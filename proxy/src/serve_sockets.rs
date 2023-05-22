use std::{time::{SystemTime, Duration}, pin::Pin, task::Poll, io::Write};

use axum::{Router, routing::{get, post}, response::{Response, IntoResponse}, extract::{State, Path}};
use chacha20poly1305::{AeadCore, KeyInit, ChaCha20Poly1305, aead::{OsRng, stream::{StreamLE31, NewStream, StreamPrimitive}, Nonce, generic_array::GenericArray}, XChaCha20Poly1305, consts::{U20, U32}};
use hyper::{Request, Body, StatusCode, upgrade::OnUpgrade};
use rsa::rand_core::RngCore;
use shared::{http_client::SamplyHttpClient, config, MsgSocketRequest, beam_id::AppOrProxyId, MsgId, Plain, MsgEmpty};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::warn;

use crate::{serve_tasks::{handler_task, TasksState, forward_request}, auth::AuthenticatedApp};


pub(crate) fn router(client: SamplyHttpClient) -> Router {
    let config = config::CONFIG_PROXY.clone();
    let state = TasksState {
        client: client.clone(),
        config,
    };
    Router::new()
        .route("/v1/sockets", get(handler_task).post(handler_task))
        .route("/v1/sockets/:app_or_id", post(create_socket_con).get(connect_socket))
        .with_state(state)
}

async fn create_socket_con(
    AuthenticatedApp(sender): AuthenticatedApp,
    Path(to): Path<AppOrProxyId>,
    state: State<TasksState>,
    req: Request<Body>
) -> Response {
    let task_id = MsgId::new();
    // TODO: proper secrets and encryption
    let secret = "";
    let socket_req = MsgSocketRequest {
        from: AppOrProxyId::AppId(sender.clone()),
        to: vec![to],
        expire: SystemTime::now() + Duration::from_secs(60),
        id: task_id,
        secret: Plain::from(secret),
        metadata: serde_json::Value::Null,
    };

    let Ok(body) = serde_json::to_vec(&socket_req) else {
        warn!("Failed to serialize MsgSocketRequest");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let new_req = Request::post("/v1/sockets")
        .body(Body::from(body));
    let post_socket_task_req = match new_req {
        Ok(req) => req,
        Err(e) => {
            warn!("Failed to construct request: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        },
    };

    let res = match forward_request(post_socket_task_req, &state.config, &sender, &state.client).await {
        Ok(res) => res,
        Err(err) => {
            warn!("Failed to create post socket request: {err:?}");
            return err.into_response()
        },
    };

    if res.status() != StatusCode::CREATED {
        warn!("Failed to post MsgSocketRequest to broker. Statuscode: {}", res.status());
        return (res.status(), "Failed to post MsgSocketRequest to broker").into_response();
    }
    connect_socket(AuthenticatedApp(sender), state, task_id, req).await
}

async fn connect_socket(
    AuthenticatedApp(sender): AuthenticatedApp,
    state: State<TasksState>,
    task_id: MsgId,
    req: Request<Body>
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
        },
    };
    *get_socket_con_req.headers_mut() = req.headers().clone();

    let res = match forward_request(get_socket_con_req, &state.config, &sender, &state.client).await {
        Ok(res) => res,
        Err(err) => {
            warn!("Failed to create socket connect request: {err:?}");
            return err.into_response()
        },
    };

    if res.extensions().get::<OnUpgrade>().is_none() || res.status() != StatusCode::SWITCHING_PROTOCOLS {
        warn!("Failed to create an upgradable connection to the broker. Response was: {res:?}");
        return res.status().into_response();
    }

    // Connect sockets
    tokio::spawn(async move {
        // TODO: Encrypt
        let mut c1 = hyper::upgrade::on(res).await.unwrap();
        let mut c2 = hyper::upgrade::on(req).await.unwrap();

        let result = tokio::io::copy_bidirectional(&mut c1, &mut c2).await;
        if let Err(e) = result {
            warn!("Error relaying socket connect: {e}");
        }
    });

    StatusCode::SWITCHING_PROTOCOLS.into_response()
}

struct EncryptedSocket<S> {
    inner: S,
    cipher: StreamLE31<XChaCha20Poly1305>,
    read_counter: u32,
    write_counter: u32
}

impl<S> EncryptedSocket<S> {
    /// Creates a new cipher stream with a 32 byte key and a 16 + 4 byte Nonce
    fn new(inner: S, key: &GenericArray<u8, U32>) -> Self {
        let mut nonce = GenericArray::<u8, U20>::default();
        OsRng.fill_bytes(&mut nonce);
        let aead = XChaCha20Poly1305::new(key);
        let cipher = StreamLE31::from_aead(aead, &nonce);
        Self { inner, cipher, read_counter: 0, write_counter: 0 }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for EncryptedSocket<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let initial_buf_len = buf.filled().len();
        let stream = Pin::new(&mut self.inner);
        let res = stream.poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = res {
            // We got some data to decrypt
            // Maybe make a PR to aead to impl aead::Buffer for tokio::io::ReadBuf to enable in place encryption or write a wrapper type
            // let result = self.cipher.decrypt_in_place(self.counter, false, b"", buf.filled_mut());
            match self.cipher.decrypt(self.read_counter, initial_buf_len == buf.filled().len(), buf.filled()) {
                Ok(plain) => {
                    self.read_counter += 1;
                    let len = buf.filled().len();
                    buf.set_filled(len);
                    buf.filled_mut().write_all(&plain).expect("Should work maybe");
                },
                Err(_) => todo!(),
            }
        }
        res
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for EncryptedSocket<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let ciphertext = self.cipher.encrypt(self.write_counter, false, buf).unwrap();
        self.write_counter += 1;
        let stream = Pin::new(&mut self.inner);
        stream.poll_write(cx, &ciphertext)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), std::io::Error>> {
        let stream = Pin::new(&mut self.inner);
        stream.poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), std::io::Error>> {
        let stream = Pin::new(&mut self.inner);
        stream.poll_shutdown(cx)
    }
}

#[tokio::test]
async fn test_encryption() {
    let mut key = GenericArray::default();
    OsRng.fill_bytes(&mut key);
    let socket_mock = tokio_test::io::Builder::new()
        .write(b"Hello world")
        .read(b"Hello world")
        .build();

    let encrypted_socket = EncryptedSocket::new(socket_mock, &key);
    // TODO: Finish
}
