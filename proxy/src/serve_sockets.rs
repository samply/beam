use std::{
    io::{self, Write},
    ops::{Deref, DerefMut},
    pin::Pin,
    task::Poll,
    time::{Duration, SystemTime},
};

use axum::{
    extract::{Path, State},
    response::{IntoResponse, Response},
    routing::{get, post},
    RequestPartsExt, Router,
};
use bytes::{Buf, BufMut, BytesMut};
use chacha20poly1305::{
    aead::{
        self,
        generic_array::{typenum::Unsigned, GenericArray},
        stream::{DecryptorLE31, EncryptorLE31, NewStream, StreamLE31, StreamPrimitive},
        Buffer, Nonce, OsRng,
    },
    consts::{U20, U32},
    AeadCore, AeadInPlace, ChaCha20Poly1305, KeyInit, XChaCha20Poly1305,
};
use futures::{stream::IntoAsyncRead, FutureExt, SinkExt, StreamExt, TryStreamExt};
use hyper::{upgrade::OnUpgrade, Body, Request, StatusCode};
use rsa::rand_core::RngCore;
use serde::{Deserialize, Serialize};
use shared::{
    beam_id::AppOrProxyId,
    config,
    ct_codecs::{Base64UrlSafeNoPadding, Encoder as B64Encoder, Decoder as B64Decoder, self},
    http_client::SamplyHttpClient,
    MsgEmpty, MsgId, MsgSocketRequest, Plain,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf, ReadHalf, WriteHalf};
use tokio_util::{
    codec::{Decoder, Encoder, Framed, FramedRead, FramedWrite},
    compat::{Compat, FuturesAsyncReadCompatExt},
};
use tracing::warn;

use crate::{
    auth::AuthenticatedApp,
    serve_tasks::{forward_request, handler_task, TasksState},
};

pub(crate) fn router(client: SamplyHttpClient) -> Router {
    let config = config::CONFIG_PROXY.clone();
    let state = TasksState {
        client: client.clone(),
        config,
    };
    Router::new()
        .route("/v1/sockets", get(handler_task).post(handler_task))
        .route("/v1/sockets/:app_or_id", post(create_socket_con))
        .route("/v1/sockets/:app_or_id/:secret", get(connect_socket))
        .with_state(state)
}

async fn create_socket_con(
    AuthenticatedApp(sender): AuthenticatedApp,
    Path(to): Path<AppOrProxyId>,
    state: State<TasksState>,
    req: Request<Body>,
) -> Response {
    let task_id = MsgId::new();
    // TODO: proper secrets and encryption
    let secret = SocketEncKey::generate();
    let Ok(secret_encoded) = secret.to_b64_str() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let socket_req = MsgSocketRequest {
        from: AppOrProxyId::AppId(sender.clone()),
        to: vec![to],
        expire: SystemTime::now() + Duration::from_secs(60),
        id: task_id,
        secret: Plain::from(secret_encoded),
        metadata: serde_json::Value::Null,
    };

    let Ok(body) = serde_json::to_vec(&socket_req) else {
        warn!("Failed to serialize MsgSocketRequest");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let new_req = Request::post("/v1/sockets").body(Body::from(body));
    let post_socket_task_req = match new_req {
        Ok(req) => req,
        Err(e) => {
            warn!("Failed to construct request: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let res =
        match forward_request(post_socket_task_req, &state.config, &sender, &state.client).await {
            Ok(res) => res,
            Err(err) => {
                warn!("Failed to create post socket request: {err:?}");
                return err.into_response();
            }
        };

    if res.status() != StatusCode::CREATED {
        warn!(
            "Failed to post MsgSocketRequest to broker. Statuscode: {}",
            res.status()
        );
        return (res.status(), "Failed to post MsgSocketRequest to broker").into_response();
    }
    connect_socket(AuthenticatedApp(sender), state, Path((task_id, secret)), req).await
}

async fn connect_socket(
    AuthenticatedApp(sender): AuthenticatedApp,
    state: State<TasksState>,
    Path((task_id, key)): Path<(MsgId, SocketEncKey)>,
    req: Request<Body>,
) -> Response {
    if req.extensions().get::<OnUpgrade>().is_none() {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    }

    let msg_empty = MsgEmpty {
        from: AppOrProxyId::AppId(sender.clone()),
    };
    let Ok(body) = serde_json::to_vec(&msg_empty) else {
        warn!("Failed to serialize MsgEmpty");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    // Try to connect to socket
    let new_req = Request::get(format!("/v1/sockets/{task_id}")).body(Body::from(body));
    let mut get_socket_con_req = match new_req {
        Ok(req) => req,
        Err(e) => {
            warn!("Failed to construct request: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    *get_socket_con_req.headers_mut() = req.headers().clone();

    let res = match forward_request(get_socket_con_req, &state.config, &sender, &state.client).await
    {
        Ok(res) => res,
        Err(err) => {
            warn!("Failed to create socket connect request: {err:?}");
            return err.into_response();
        }
    };

    if res.extensions().get::<OnUpgrade>().is_none()
        || res.status() != StatusCode::SWITCHING_PROTOCOLS
    {
        warn!("Failed to create an upgradable connection to the broker. Response was: {res:?}");
        return res.status().into_response();
    }

    // Connect sockets
    tokio::spawn(async move {
        let broker_socket = hyper::upgrade::on(res).await.unwrap();
        let mut client_socket = hyper::upgrade::on(req).await.unwrap();
        let Ok(mut enc_broker_socket) = EncryptedSocket::new(broker_socket, &key).await else {
            warn!("Error establishing connection");
            return;
        };

        let result = tokio::io::copy_bidirectional(&mut client_socket, &mut enc_broker_socket).await;
        if let Err(e) = result {
            warn!("Error relaying socket connect: {e}");
        }
    });

    StatusCode::SWITCHING_PROTOCOLS.into_response()
}

struct SocketEncKey(GenericArray<u8, U32>);

impl SocketEncKey {
    fn generate() -> Self {
        let mut arr = GenericArray::default();
        OsRng.fill_bytes(&mut arr);
        SocketEncKey(arr)
    }

    fn to_b64_str(&self) -> Result<String, ct_codecs::Error> {
        Base64UrlSafeNoPadding::encode_to_string(self.as_slice())
    }
}

impl Serialize for SocketEncKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_b64_str().map_err(serde::ser::Error::custom)?.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SocketEncKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = Base64UrlSafeNoPadding::decode_to_vec(String::deserialize(deserializer)?, None).map_err(serde::de::Error::custom)?;
        if bytes.len() != U32::to_usize() {
            return Err(serde::de::Error::custom("Key does not match required key length"));
        } else {
            Ok(SocketEncKey(GenericArray::clone_from_slice(bytes.as_slice())))
        }
    }
}

impl Deref for SocketEncKey {
    type Target = GenericArray<u8, U32>;

    fn deref(&self) -> &GenericArray<u8, U32> {
        &self.0
    }
}

impl DerefMut for SocketEncKey {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

struct EncryptedSocket<S: AsyncRead + AsyncWrite> {
    // inner: Framed<S, EncryptorCodec>,
    read: Compat<IntoAsyncRead<FramedRead<ReadHalf<S>, DecryptorCodec>>>,
    write: FramedWrite<WriteHalf<S>, EncryptorCodec>,
}

struct EncryptorCodec {
    encryptor: EncryptorLE31<XChaCha20Poly1305>,
}

struct DecryptorCodec {
    decryptor: DecryptorLE31<XChaCha20Poly1305>,
}

impl DecryptorCodec {
    const SIZE_OVERHEAD: usize = 4;
}

impl EncryptorCodec {
    #[inline]
    fn tag_overhead() -> usize {
        <XChaCha20Poly1305 as AeadCore>::TagSize::to_usize()
    }
}

impl Encoder<&[u8]> for EncryptorCodec {
    type Error = io::Error;

    fn encode(&mut self, item: &[u8], dst: &mut BytesMut) -> Result<(), Self::Error> {
        let mut enc_buf = EncBuffer::new(dst, item.len() + Self::tag_overhead());
        enc_buf.extend_from_slice(item).expect("Infallible");
        self.encryptor
            .encrypt_next_in_place(b"", &mut enc_buf)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Encryption failed"))
    }
}

impl Decoder for DecryptorCodec {
    type Item = Vec<u8>;

    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < Self::SIZE_OVERHEAD {
            return Ok(None);
        }
        let mut size_slice = [0; Self::SIZE_OVERHEAD];
        size_slice.clone_from_slice(&src[..Self::SIZE_OVERHEAD]);
        let size = u32::from_le_bytes(size_slice);
        let total_frame_size = size as usize + Self::SIZE_OVERHEAD;
        if src.len() < total_frame_size {
            return Ok(None);
        }

        let plain = self
            .decryptor
            .decrypt_next(&src[Self::SIZE_OVERHEAD..])
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Decryption failed"))?;
        src.advance(total_frame_size);
        Ok(Some(plain))
    }
}

struct EncBuffer<'a> {
    buf: &'a mut BytesMut,
}

impl<'a> EncBuffer<'a> {
    fn new(buffer: &'a mut BytesMut, content_len: usize) -> Self {
        buffer.reserve(content_len + Self::SIZE_OVERHEAD);
        buffer.extend_from_slice(&u32::to_le_bytes(content_len as u32));
        Self { buf: buffer }
    }

    /// Reserved for size of msg
    const SIZE_OVERHEAD: usize = 4;
}

impl<'a> Buffer for EncBuffer<'a> {
    // This should only be called to append the tag to the buffer
    fn extend_from_slice(&mut self, other: &[u8]) -> aead::Result<()> {
        self.buf.extend_from_slice(other);
        Ok(())
    }

    // This should only be called when decrypting
    fn truncate(&mut self, len: usize) {
        self.buf.truncate(len)
    }
}

impl<'a> AsRef<[u8]> for EncBuffer<'a> {
    fn as_ref(&self) -> &[u8] {
        &self.buf[Self::SIZE_OVERHEAD..]
    }
}

impl<'a> AsMut<[u8]> for EncBuffer<'a> {
    fn as_mut(&mut self) -> &mut [u8] {
        &mut self.buf[Self::SIZE_OVERHEAD..]
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> EncryptedSocket<S> {
    /// Creates a new cipher stream with a 32 byte key and a 16 + 4 byte Nonce
    async fn new(mut inner: S, key: &GenericArray<u8, U32>) -> io::Result<Self> {
        let aead = XChaCha20Poly1305::new(key);

        // Encryption
        let mut enc_nonce = GenericArray::default();
        OsRng.fill_bytes(&mut enc_nonce);
        let encryptor = EncryptorLE31::from_aead(aead.clone(), &enc_nonce);
        inner.write_all(&enc_nonce).await?;

        // Decryption
        let mut dec_nonce = GenericArray::default();
        inner.read_exact(dec_nonce.as_mut_slice()).await?;
        let decryptor = DecryptorLE31::from_aead(aead, &dec_nonce);

        let (r, w) = tokio::io::split(inner);
        let read = FramedRead::new(r, DecryptorCodec { decryptor });
        let read = read.into_async_read().compat();
        let write = FramedWrite::new(w, EncryptorCodec { encryptor });

        Ok(Self { read, write })
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRead for EncryptedSocket<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        Pin::new(&mut self.read).poll_read(cx, buf)
        // self.read.poll_next_unpin(cx).map(|item| match item {
        //     Some(Ok(plain)) => {
        //         buf.put_slice(&plain);
        //         Ok(())
        //     },
        //     Some(Err(e)) => Err(e),
        //     None => Ok(())
        // })
    }
}

impl<S: AsyncWrite + AsyncRead + Unpin> AsyncWrite for EncryptedSocket<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, io::Error>> {
        self.write.send(buf).poll_unpin(cx).map_ok(|_| buf.len())
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), io::Error>> {
        self.write.poll_flush_unpin(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), io::Error>> {
        self.write.poll_close_unpin(cx)
    }
}

#[cfg(test)]
mod tests {
    use chacha20poly1305::aead::stream::{Decryptor, Encryptor, EncryptorLE31};
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    #[tokio::test]
    async fn test_encryption() {
        let mut key = GenericArray::default();
        OsRng.fill_bytes(&mut key);
        const N: usize = 2_usize.pow(13);
        let test_data: &mut [u8; N] = &mut [0; N];
        OsRng.fill_bytes(test_data);
        let mut read_buf = [0; N];

        start_test_broker().await;
        let (mut client1, mut client2) = tokio::join!(client(&key), client(&key));

        client1.write_all(test_data).await.unwrap();
        client2.read_exact(&mut read_buf).await.unwrap();
        assert_eq!(test_data, &read_buf);
        client2.write_all(test_data).await.unwrap();
        client1.read_exact(&mut read_buf).await.unwrap();
        assert_eq!(test_data, &read_buf);
    }

    async fn start_test_broker() {
        let server = TcpListener::bind("127.0.0.1:1337").await.unwrap();
        tokio::spawn(async move {
            let ((mut a, _), (mut b, _)) = tokio::try_join!(server.accept(), server.accept()).unwrap();
            tokio::io::copy_bidirectional(&mut a, &mut b).await.unwrap();
        });
    }

    async fn client(key: &GenericArray<u8, U32>) -> impl AsyncRead + AsyncWrite {
        // Wait for server to start
        tokio::time::sleep(Duration::from_millis(100)).await;
        let stream = TcpStream::connect("127.0.0.1:1337").await.unwrap();
        EncryptedSocket::new(stream, key).await.unwrap()
    }

    #[test]
    fn normal_enc() {
        let mut key = GenericArray::default();
        OsRng.fill_bytes(&mut key);
        const N: usize = 2_usize.pow(10);
        let test_data: &mut [u8; N] = &mut [0; N];
        OsRng.fill_bytes(test_data);

        let mut nonce = GenericArray::<u8, U20>::default();
        OsRng.fill_bytes(&mut nonce);
        let aead = XChaCha20Poly1305::new(&key);
        let client1 = StreamLE31::from_aead(aead, &nonce);
        let mut encrypter = Encryptor::from_stream_primitive(client1);

        // let mut nonce = GenericArray::<u8, U20>::default();
        // OsRng.fill_bytes(&mut nonce);
        let aead = XChaCha20Poly1305::new(&key);
        let client2 = StreamLE31::from_aead(aead, &nonce);
        let mut decrypter = Decryptor::from_stream_primitive(client2);

        let cipher_text = encrypter.encrypt_next(test_data.as_slice()).unwrap();
        dbg!(cipher_text.len());
        let a = decrypter.decrypt_next(cipher_text.as_slice()).unwrap();
        assert_eq!(test_data, a.as_slice());
    }
}
