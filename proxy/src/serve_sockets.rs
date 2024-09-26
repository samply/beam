use std::{
    future::ready, sync::Arc, task::Poll, time::{Duration, SystemTime}
};

use axum::{
    body::Body, extract::{Path, Request, State}, http::{self, header, HeaderMap, HeaderValue, StatusCode}, response::{IntoResponse, Response}, routing::{get, post}, Extension, Json, RequestPartsExt, Router
};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use crypto_secretstream::{Header, Key, PullStream, PushStream};
use dashmap::DashMap;
use futures::{stream::{self, IntoAsyncRead}, FutureExt, SinkExt, Stream, StreamExt, TryStream, TryStreamExt};
use rsa::rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use beam_lib::AppOrProxyId;
use shared::{
    config, ct_codecs::{self, Base64UrlSafeNoPadding, Decoder as B64Decoder, Encoder as B64Encoder}, expire_map::LazyExpireMap, http_client::SamplyHttpClient, reqwest, MessageType, MsgEmpty, MsgId, MsgSocketRequest, Plain
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf, ReadHalf, WriteHalf};
use tracing::{warn, debug};

use crate::{
    auth::AuthenticatedApp,
    serve_tasks::{forward_request, handler_task, prepare_request, to_server_error, validate_and_decrypt, TasksState},
};

type MsgSecretMap = Arc<LazyExpireMap<MsgId, Key>>;
const TASK_SECRET_CLEANUP_INTERVAL: Duration = Duration::from_secs(5 * 60);

pub(crate) fn router(client: SamplyHttpClient) -> Router {
    let config = config::CONFIG_PROXY.clone();
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
        .route("/v1/sockets/:app_or_id", post(create_socket_con).get(connect_read))
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
    let plain_json = to_server_error(validate_and_decrypt(enc_json).await).map_err(IntoResponse::into_response)?;
    let tasks: Vec<MessageType<Plain>> = serde_json::from_value(plain_json).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())?;
    let mut out = Vec::with_capacity(tasks.len());
    for task in tasks {
        if let MessageType::MsgSocketRequest(mut socket_task) = task {
            let key = socket_task.secret.body
                .as_ref()
                .and_then(|s| Base64UrlSafeNoPadding::decode_to_vec(s, None).ok())
                .and_then(|b| Key::try_from(b.as_slice()).ok())
                .ok_or_else(|| {
                    warn!("Failed to extract socket decryption key");
                    StatusCode::INTERNAL_SERVER_ERROR.into_response()
                })?;
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
    mut og_req: Request,
) -> Response {
    let task_id = MsgId::new();
    let secret = Key::generate(&mut OsRng);
    let Ok(secret_encoded) = Base64UrlSafeNoPadding::encode_to_string(secret.as_ref()) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    const TTL: Duration = Duration::from_secs(60);
    task_secret_map.insert_for(TTL, task_id.clone(), secret);
    let metadata = og_req
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
    let post_socket_task_req = Request::post("/v1/sockets").body(axum::body::Body::from(body)).unwrap();
    let res = match forward_request(post_socket_task_req, &state.config, &sender, &state.client).await {
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
    let req = match prepare_socket_request(sender, task_id, &state).await {
        Ok(req) => req,
        Err(e) => return e,
    };
    let req = req.map(|b| {
        let n = b.as_bytes().len();
        let mut body = Vec::with_capacity(n + 5);
        // This 0 signals write interest
        body.push(0);
        body.extend(u32::to_be_bytes(n as _));
        body.extend(b.as_bytes());
        Bytes::from(body)
    });

    let Some(key) = task_secret_map.remove(&task_id) else {
        return StatusCode::GONE.into_response();
    };
    let (mut parts, body) = req.into_parts();
    parts.headers.append(header::TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
    let stream = stream::once(ready(Ok(body))).chain(Encrypter::new(key).encrypt(og_req.into_body().into_data_stream()));
    let req = Request::from_parts(parts, reqwest::Body::wrap_stream(stream));
    match state.client.execute(req.try_into().expect("Conversion to reqwest::Request should always work")).await {
        Ok(res) => http::Response::try_from(res).expect("reqwest::Response to http::Response should always work").map(Body::new),
        Err(e) => {
            warn!("Failed to stream data to broker: {e}");
            (StatusCode::BAD_GATEWAY, e.to_string()).into_response()
        },
    }
}

async fn connect_read(
    AuthenticatedApp(sender): AuthenticatedApp,
    state: State<TasksState>,
    Extension(task_secret_map): Extension<MsgSecretMap>,
    Path(task_id): Path<MsgId>,
) -> Response {
    let req = match prepare_socket_request(sender, task_id, &state).await {
        Ok(value) => value,
        Err(e) => return e,
    };
    let req = req.map(|b| {
        let n = b.as_bytes().len();
        let mut body = Vec::with_capacity(n + 4 + 1);
        // This 1 signals read interest
        body.push(1);
        body.extend(u32::to_be_bytes(n as _));
        body.extend(b.as_bytes());
        Bytes::from(body)
    });

    let res = match state.client.execute(req.try_into().expect("Conversion to reqwest::Request should always work")).await {
        Ok(res) => res,
        Err(e) => {
            warn!("Failed to read from broker: {e}");
            return (StatusCode::BAD_GATEWAY, e.to_string()).into_response()
        },
    };
    let Some(key) = task_secret_map.remove(&task_id) else {
        return StatusCode::GONE.into_response();
    };
    Response::new(Body::from_stream(Decrypter::new(key).decrypt(res.bytes_stream())))
}

async fn prepare_socket_request(sender: beam_lib::AppId, task_id: MsgId, state: &State<TasksState>) -> Result<http::Request<String>, http::Response<Body>> {
    let msg_empty = MsgEmpty {
        from: AppOrProxyId::App(sender.clone()),
    };
    let Ok(body) = serde_json::to_vec(&msg_empty) else {
        warn!("Failed to serialize MsgEmpty");
        return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
    };
    let new_req = Request::get(format!("/v1/sockets/{task_id}")).body(axum::body::Body::from(body));
    let get_socket_con_req = match new_req {
        Ok(req) => req,
        Err(e) => {
            warn!("Failed to construct request: {e}");
            return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    };
    match prepare_request(get_socket_con_req, &state.config, &sender).await {
        Ok(req) => Ok(req),
        Err(err) => {
            warn!("Failed to create socket connect request: {err:?}");
            Err(err.into_response())
        }
    }
}

struct Encrypter {
    header: Header,
    encrypter: PushStream,
}

impl Encrypter {
    fn new(key: Key) -> Self {
        let (header, encrypter) = PushStream::init(&mut OsRng, &key);
        Self { header, encrypter }
    }

    fn encrypt<S: Stream<Item = Result<Bytes, axum::Error>> + Send + 'static>(mut self, stream: S) -> impl Stream<Item = Result<Bytes, axum::Error>> + Send + 'static {
        let header = Bytes::copy_from_slice(self.header.as_ref().as_slice());
        stream::once(futures::future::ready(Ok(header))).chain(stream.map_ok(move |bytes| {
            let mut buf = bytes.to_vec();
            self.encrypter.push(&mut buf, b"", crypto_secretstream::Tag::Message).expect("Hope this works");
            let mut dst = Vec::with_capacity(buf.len() + 4);
            dst.extend(u32::to_be_bytes(buf.len() as _));
            dst.extend(buf);
            dst.into()
        }))
    }
}

struct Decrypter {
    key: Key,
}

impl Decrypter {
    fn new(key: Key) -> Self {
        Self { key }
    }

    fn decrypt<S: Stream<Item = reqwest::Result<Bytes>> + Send + Unpin + 'static>(self, mut stream: S) -> impl Stream<Item = reqwest::Result<Bytes>> + Send + 'static {
        enum State {
            ReadingHeader,
            ReadingLen(PullStream),
            Decrypting(PullStream, usize)
        }
        let mut buf = BytesMut::new();
        let mut state = State::ReadingHeader;
        futures::stream::poll_fn(move |cx| {
            loop {
                return match &mut state {
                    State::ReadingHeader if buf.len() >= Header::BYTES => {
                        let head = buf.split_to(Header::BYTES);
                        state = State::ReadingLen(PullStream::init(head.as_ref().try_into().unwrap(), &self.key));
                        continue;
                    },
                    State::ReadingLen(..) if buf.len() >= 4 => {
                        let State::ReadingLen(dec) = std::mem::replace(&mut state, State::ReadingHeader) else {
                            unreachable!()
                        };
                        let len = buf.split_to(4);
                        state = State::Decrypting(dec, u32::from_be_bytes(len.as_ref().try_into().unwrap()) as usize);
                        continue;
                    },
                    State::Decrypting(ref mut dec, len) if buf.len() >= *len => {
                        let mut data = buf.split_to(*len).to_vec();
                        assert_eq!(dec.pull(&mut data, b""), Ok(crypto_secretstream::Tag::Message));
                        let State::Decrypting(dec, _) = std::mem::replace(&mut state, State::ReadingHeader) else {
                            unreachable!()
                        };
                        state = State::ReadingLen(dec);
                        Poll::Ready(Some(Ok(Bytes::from(data))))
                    },
                    _ => match futures::ready!(stream.poll_next_unpin(cx)) {
                        Some(Ok(data)) => {
                            buf.extend(data);
                            continue;
                        },
                        Some(Err(e)) => Poll::Ready(Some(Err(e))),
                        None => Poll::Ready(None),
                    }
                }
            }
        })
    }
}
