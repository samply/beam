use std::{
    io, mem::size_of, pin::Pin, sync::{atomic::AtomicUsize, Arc}, time::{Duration, Instant, SystemTime}
};

use axum::{
    extract::{Path, Request, State}, http::{self, header, HeaderValue, StatusCode}, response::{IntoResponse, Response}, routing::{get, post}, Extension, Json, RequestPartsExt, Router
};
use aead_stream::{DecryptorLE31, EncryptorLE31, Nonce, StreamLE31};
use bytes::{Buf, BufMut, BytesMut};
use chacha20poly1305::{aead::{array::typenum::Unsigned, AeadCore, Generate}, Key, KeyInit, XChaCha20Poly1305};
use dashmap::DashMap;
use futures::{stream::IntoAsyncRead, FutureExt, SinkExt, StreamExt, TryStreamExt};
use hyper_util::rt::TokioIo;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use beam_lib::AppOrProxyId;
use shared::{
    ct_codecs::{self, Base64UrlSafeNoPadding, Decoder as B64Decoder, Encoder as B64Encoder}, expire_map::LazyExpireMap, http_client::SamplyHttpClient, reqwest, MessageType, MsgEmpty, MsgId, MsgSocketRequest, Plain
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf, ReadHalf, WriteHalf};
use tokio_util::{
    codec::{Decoder, Encoder, Framed, FramedRead, FramedWrite},
    compat::{Compat, FuturesAsyncReadCompatExt}, io::SinkWriter,
};
use tracing::{warn, debug};

use crate::{
    auth::AuthenticatedApp, config::Config, serve_tasks::{forward_request, handler_task, to_server_error, validate_and_decrypt, TasksState}
};

type MsgSecretMap = Arc<LazyExpireMap<MsgId, SocketEncKey>>;
type StreamNonce = Nonce<XChaCha20Poly1305, StreamLE31<XChaCha20Poly1305>>;
type FrameLen = u32;
const FRAME_LEN_SIZE: usize = size_of::<FrameLen>();
const TAG_SIZE: usize = <XChaCha20Poly1305 as AeadCore>::TagSize::USIZE;
const TASK_SECRET_CLEANUP_INTERVAL: Duration = Duration::from_secs(5 * 60);

pub(crate) fn router(client: SamplyHttpClient, config: &'static Config) -> Router {
    let state = TasksState {
        client: client.clone(),
        config,
    };
    let task_secret_map: MsgSecretMap = Default::default();
    let map = task_secret_map.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(TASK_SECRET_CLEANUP_INTERVAL).await;
            map.retain_expired();
        }
    });

    Router::new()
        .route("/v1/sockets", get(get_tasks))
        .route("/v1/sockets/{app_or_id}", post(create_socket_con).get(connect_socket))
        .with_state(state)
        .layer(Extension(task_secret_map))
}

async fn get_tasks(
    AuthenticatedApp(sender): AuthenticatedApp,
    state: State<TasksState>,
    Extension(task_secret_map): Extension<MsgSecretMap>,
    req: Request
) -> Result<Json<Vec<MsgSocketRequest<Plain>>>, Response> {
    let res = forward_request(req, &state.config, &sender, &state.client).await?;
    if res.status() != StatusCode::OK {
        return Err(http::Response::from(res).map(axum::body::Body::new));
    }
    let enc_json = res.json().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
    let plain_json = to_server_error(validate_and_decrypt(enc_json, state.config).await).map_err(IntoResponse::into_response)?;
    let tasks: Vec<MessageType<Plain>> = serde_json::from_value(plain_json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
    let mut out = Vec::with_capacity(tasks.len());
    for task in tasks {
        if let MessageType::MsgSocketRequest(mut socket_task) = task {
            let key = serde_json::from_value(Value::String(socket_task.secret.body
                .as_ref()
                .ok_or(StatusCode::INTERNAL_SERVER_ERROR.into_response())?
                .to_string()
            )).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
            let Ok(ttl) = socket_task.expire.duration_since(SystemTime::now()) else {
                continue;
            };
            task_secret_map.insert_for(ttl, socket_task.id, key);
            socket_task.secret.body = None;
            out.push(socket_task);
        } else {
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }

    Ok(Json(out))
}

async fn create_socket_con(
    AuthenticatedApp(sender): AuthenticatedApp,
    Path(to): Path<AppOrProxyId>,
    Extension(task_secret_map): Extension<MsgSecretMap>,
    state: State<TasksState>,
    mut req: Request,
) -> Response {
    let task_id = MsgId::new();
    let secret = SocketEncKey::generate();
    let Ok(secret_encoded) = secret.to_b64_str() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    const TTL: Duration = Duration::from_secs(60);
    task_secret_map.insert_for(TTL, task_id.clone(), secret);
    let metadata = req
        .headers_mut()
        .remove("metadata")
        .and_then(|v| serde_json::from_slice(v.as_bytes()).ok())
        .unwrap_or_default();
    let socket_req = MsgSocketRequest {
        from: AppOrProxyId::App(sender.clone()),
        to: vec![to],
        expire: SystemTime::now() + TTL,
        id: task_id,
        secret: Plain::from(secret_encoded),
        metadata
    };

    let Ok(body) = serde_json::to_vec(&socket_req) else {
        warn!("Failed to serialize MsgSocketRequest");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let new_req = Request::post("/v1/sockets").body(axum::body::Body::from(body));
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
    connect_socket(AuthenticatedApp(sender), state, Extension(task_secret_map), Path(task_id), req).await
}

async fn connect_socket(
    AuthenticatedApp(sender): AuthenticatedApp,
    state: State<TasksState>,
    Extension(task_secret_map): Extension<MsgSecretMap>,
    Path(task_id): Path<MsgId>,
    mut req: Request,
) -> Response {
    let Some(conn) =  req.extensions_mut().remove::<hyper::upgrade::OnUpgrade>() else {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    };

    let Some(key) = task_secret_map.get(&task_id).map(|v| v.clone()) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    let msg_empty = MsgEmpty {
        from: AppOrProxyId::App(sender.clone()),
    };
    let Ok(body) = serde_json::to_vec(&msg_empty) else {
        warn!("Failed to serialize MsgEmpty");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    // Try to connect to socket
    let new_req = Request::get(format!("/v1/sockets/{task_id}")).body(axum::body::Body::from(body));
    let mut get_socket_con_req = match new_req {
        Ok(req) => req,
        Err(e) => {
            warn!("Failed to construct request: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    get_socket_con_req.headers_mut().insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
    get_socket_con_req.headers_mut().insert(header::UPGRADE, HeaderValue::from_static("tcp"));

    let mut res = match forward_request(get_socket_con_req, &state.config, &sender, &state.client).await
    {
        Ok(res) => res,
        Err(err) => {
            warn!("Failed to create socket connect request: {err:?}");
            return err.into_response();
        }
    };

    let broker_conn = match res.extensions_mut().remove::<hyper::upgrade::OnUpgrade>() {
        Some(other_conn) if res.status() == StatusCode::SWITCHING_PROTOCOLS => other_conn,
        _ => {
            let s = res.status();
            let res = res.text().await.unwrap_or_else(|_| "<Failed to read body>".into());
            warn!("Failed to create an upgradable connection to the broker. {s}: {res}");
            return s.into_response();
        }
    };

    // Connect sockets
    tokio::spawn(async move {
        let (broker_socket, client_socket) = match tokio::try_join!(broker_conn, conn) {
            Ok(sockets) => sockets,
            Err(e) => {
                warn!("Failed to upgrade requests to socket connections: {e}");
                return;
            },
        };
        let Ok(mut enc_broker_socket) = EncryptedSocket::new(TokioIo::new(broker_socket), &key).await else {
            warn!("Error encrypting connection to broker");
            return;
        };

        let result = tokio::io::copy_bidirectional(&mut TokioIo::new(client_socket), &mut enc_broker_socket).await;
        if let Err(e) = result {
            debug!("Relaying socket connection ended: {e}");
        }
    });

    ([
        (header::UPGRADE, HeaderValue::from_static("tcp")),
        (header::CONNECTION, HeaderValue::from_static("upgrade"))
    ], StatusCode::SWITCHING_PROTOCOLS).into_response()
}

#[derive(Debug, Clone, Copy)]
struct SocketEncKey(Key);

impl SocketEncKey {
    fn generate() -> Self {
        SocketEncKey(Key::generate())
    }

    fn to_b64_str(&self) -> Result<String, ct_codecs::Error> {
        Base64UrlSafeNoPadding::encode_to_string(self.0.as_slice())
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
        let key = Key::try_from(bytes.as_slice())
            .map_err(|_| serde::de::Error::custom("Key does not match required key length"))?;
        Ok(SocketEncKey(key))
    }
}

struct EncryptedSocket<S: AsyncRead + AsyncWrite> {
    // inner: Framed<S, EncryptorCodec>,
    read: Compat<IntoAsyncRead<FramedRead<ReadHalf<S>, DecryptorCodec>>>,
    write: SinkWriter<FramedWrite<WriteHalf<S>, EncryptorCodec>>,
}

struct EncryptorCodec {
    encryptor: EncryptorLE31<XChaCha20Poly1305>,
}

struct DecryptorCodec {
    decryptor: DecryptorLE31<XChaCha20Poly1305>,
}

impl Encoder<&[u8]> for EncryptorCodec {
    type Error = io::Error;

    fn encode(&mut self, item: &[u8], dst: &mut BytesMut) -> Result<(), Self::Error> {
        let frame_len: FrameLen = item
            .len()
            .checked_add(TAG_SIZE)
            .and_then(|l| l.try_into().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "item too large"))?;

        dst.reserve(FRAME_LEN_SIZE + frame_len as usize);
        let frame_start = dst.len();
        dst.put_u32_le(frame_len);
        dst.extend_from_slice(item);
        let mut frame = dst.split_off(frame_start + FRAME_LEN_SIZE);
        self
            .encryptor
            .encrypt_next_in_place(&[], &mut frame)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Encryption failed"))?;
        dst.unsplit(frame);
        Ok(())
    }
}

impl Decoder for DecryptorCodec {
    type Item = BytesMut;

    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        if src.len() < FRAME_LEN_SIZE {
            return Ok(None);
        }
        let mut size_slice = [0; FRAME_LEN_SIZE];
        size_slice.copy_from_slice(&src[..FRAME_LEN_SIZE]);
        let size = FrameLen::from_le_bytes(size_slice) as usize;
        let total_frame_size = size + FRAME_LEN_SIZE;
        if src.len() < total_frame_size {
            return Ok(None);
        }

        src.advance(FRAME_LEN_SIZE);
        let mut frame = src.split_to(size);
        self
            .decryptor
            .decrypt_next_in_place(&[], &mut frame)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Decryption failed"))?;
        Ok(Some(frame))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> EncryptedSocket<S> {
    /// Creates a new cipher stream from a shared AEAD key and per-direction STREAM nonces.
    async fn new(mut inner: S, key: &SocketEncKey) -> io::Result<Self> {
        let cipher = XChaCha20Poly1305::new(&key.0);

        // Encryption
        let enc_nonce = StreamNonce::generate();
        let encryptor = EncryptorLE31::from_aead(cipher.clone(), &enc_nonce);
        inner.write_all(&enc_nonce).await?;

        // Decryption
        let mut dec_nonce = StreamNonce::default();
        inner.read_exact(dec_nonce.as_mut_slice()).await?;
        let decryptor = DecryptorLE31::from_aead(cipher, &dec_nonce);

        let (r, w) = tokio::io::split(inner);
        let read = FramedRead::new(r, DecryptorCodec { decryptor });
        let read = read.into_async_read().compat();
        let write = SinkWriter::new(FramedWrite::new(w, EncryptorCodec { encryptor }));

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
    }
}

impl<S: AsyncWrite + AsyncRead + Unpin> AsyncWrite for EncryptedSocket<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.write).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), io::Error>> {
        Pin::new(&mut self.write).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), io::Error>> {
        Pin::new(&mut self.write).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use rand::RngExt;
    use tokio::net::{TcpListener, TcpStream};

    use super::*;

    #[tokio::test]
    async fn test_encryption() {
        let key = SocketEncKey::generate();
        let data: Arc<Vec<_>> = (0..13337).map(|_| {
            let mut chunk = vec![0; rand::rng().random_range(1..9999)];
            rand::rng().fill(&mut chunk[..]);
            chunk
        }).collect::<Vec<_>>().into();

        start_test_broker().await;
        let (mut client1, mut client2) = tokio::join!(client(&key), client(&key));

        let data_cp = data.clone();
        let a = tokio::spawn(async move {
            for c in data_cp.iter() {
                client1.write_all(&c).await.unwrap();
                client1.flush().await.unwrap();
            }
        });
        let data_cp = data.clone();
        let b = tokio::spawn(async move {
            for (i, c) in data_cp.iter().enumerate() {
                let mut buf = vec![0; c.len()];
                client2.read_exact(&mut buf).await.unwrap();
                if &buf != c {
                    let mut remaining = Vec::new();
                    client2.read_to_end(&mut remaining).await.unwrap();
                    assert_eq!(&buf, c, "{i}: {remaining:?}");
                }
            }
        });
        tokio::try_join!(a, b).unwrap();
    }

    async fn start_test_broker() {
        let server = TcpListener::bind("127.0.0.1:1337").await.unwrap();
        tokio::spawn(async move {
            let ((mut a, _), (mut b, _)) = tokio::try_join!(server.accept(), server.accept()).unwrap();
            tokio::io::copy_bidirectional(&mut a, &mut b).await.unwrap();
        });
    }

    async fn client(key: &SocketEncKey) -> impl AsyncRead + AsyncWrite {
        // Wait for server to start
        tokio::time::sleep(Duration::from_millis(100)).await;
        let stream = TcpStream::connect("127.0.0.1:1337").await.unwrap();
        EncryptedSocket::new(stream, key).await.unwrap()
    }

    #[test]
    fn normal_enc() {
        const N: usize = 2_usize.pow(10);
        let test_data: &mut [u8; N] = &mut [0; N];
        rand::rng().fill(&mut test_data[..]);

        let key = SocketEncKey::generate();
        let cipher = XChaCha20Poly1305::new(&key.0);
        let nonce = StreamNonce::generate();
        let mut encryptor = EncryptorLE31::from_aead(cipher.clone(), &nonce);
        let mut decryptor = DecryptorLE31::from_aead(cipher, &nonce);
        let cipher_text = encryptor.encrypt_next(test_data.as_slice()).unwrap();
        let plain = decryptor.decrypt_next(cipher_text.as_slice()).unwrap();
        assert_eq!(test_data.as_slice(), plain);
    }
}
