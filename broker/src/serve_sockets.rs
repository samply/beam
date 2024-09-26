use std::{borrow::Cow, collections::{HashMap, HashSet}, ops::Deref, sync::Arc, time::Duration};

use axum::{body::{Body, BodyDataStream}, extract::{Path, Request, State}, http::{header, request::Parts, StatusCode}, response::{IntoResponse, Response}, routing::get, RequestExt, Router};
use bytes::{Buf, BufMut, Bytes, BytesMut};
use dashmap::mapref::entry::Entry;
use futures_core::TryStream;
use futures_util::{stream, StreamExt};
use serde::{Serialize, Serializer, ser::SerializeSeq};
use shared::{config::{CONFIG_CENTRAL, CONFIG_SHARED}, crypto_jwt::Authorized, errors::SamplyBeamError, expire_map::LazyExpireMap, serde_helpers::DerefSerializer, Encrypted, HasWaitId, HowLongToBlock, Msg, MsgEmpty, MsgId, MsgSigned, MsgSocketRequest};
use tokio::{sync::{broadcast::{self, Sender}, oneshot, RwLock}, time::Instant};
use tracing::{debug, log::error, warn, Span};

use crate::task_manager::{TaskManager, Task};


#[derive(Clone)]
struct SocketState {
    task_manager: Arc<TaskManager<MsgSocketRequest<Encrypted>>>,
    waiting_connections: Arc<LazyExpireMap<MsgId, ConnectionState>>
}

enum ConnectionState {
    ReadHalfConnected(oneshot::Sender<SocketStream>),
    WriteHalfConnected(oneshot::Receiver<SocketStream>),
}

type SocketStream = stream::BoxStream<'static, Result<Bytes, axum::Error>>;

impl SocketState {
    const WAITING_CONNECTIONS_TIMEOUT: Duration = Duration::from_secs(60);
    const WAITING_CONNECTIONS_CLEANUP_INTERVAL: Duration = Duration::from_secs(5 * 60);
}

impl Default for SocketState {
    fn default() -> Self {
        let waiting_connections: Arc<LazyExpireMap<_, _>> = Default::default();
        let cons = waiting_connections.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Self::WAITING_CONNECTIONS_CLEANUP_INTERVAL).await;
                cons.retain_expired();
            }
        });
        Self {
            task_manager: TaskManager::new(),
            waiting_connections
        }
    }
}

pub(crate) fn router() -> Router {
    Router::new()
        .route("/v1/sockets", get(get_socket_requests).post(post_socket_request))
        .route("/v1/sockets/:id", get(connect_socket))
        .with_state(SocketState::default())
}


async fn get_socket_requests(
    mut block: HowLongToBlock,
    state: State<SocketState>,
    msg: MsgSigned<MsgEmpty>,
) -> Result<DerefSerializer, StatusCode> {
    if block.wait_count.is_none() && block.wait_time.is_none() {
        block.wait_count = Some(1);
    }
    let requester = msg.get_from();
    let filter = |req: &MsgSocketRequest<Encrypted>| req.to.contains(requester);

    let socket_reqs = state.task_manager.wait_for_tasks(&block, filter).await?;
    DerefSerializer::new(socket_reqs, block.wait_count).map_err(|e| {
        warn!("Failed to serialize socket tasks: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

async fn post_socket_request(
    state: State<SocketState>,
    msg: MsgSigned<MsgSocketRequest<Encrypted>>,
) -> Result<impl IntoResponse, StatusCode> {
    let msg_id = msg.wait_id();
    state.task_manager.post_task(msg)?;

    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, format!("/v1/sockets/{}", msg_id))]
    ))
}

// TODO: Instrument with task_id
async fn connect_socket(
    state: State<SocketState>,
    Path(task_id): Path<MsgId>,
    mut parts: Parts,
    body: Body,
    // This Result is just an Either type. An error value does not mean something went wrong
) -> Result<Response, StatusCode> {
    let mut body_stream = body.into_data_stream();
    let (is_read, jwt, remaining) = read_header(&mut body_stream).await?;
    let result = shared::crypto_jwt::verify_with_extended_header::<MsgEmpty>(&mut parts, String::from_utf8_lossy(&jwt).as_ref()).await;
    let msg = match result {
        Ok(msg) => msg.msg,
        Err(e) => return Ok(e.into_response()),
    };
    {
        let task = state.task_manager.get(&task_id)?;
        // Allowed to connect are the issuer of the task and the recipient
        if !(task.get_from() == &msg.from || task.get_to().contains(&msg.from)) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    if is_read {
        let recv = match state.waiting_connections.entry(task_id) {
            Entry::Occupied(e) => {
                match e.remove().0 {
                    ConnectionState::ReadHalfConnected(_) => {
                        warn!("Reader connected twice");
                        return Err(StatusCode::INTERNAL_SERVER_ERROR);
                    },
                    ConnectionState::WriteHalfConnected(recv) => recv,
                }
            },
            Entry::Vacant(empty) => {
                let (tx, rx) = oneshot::channel();
                empty.insert((ConnectionState::ReadHalfConnected(tx), Instant::now() + SocketState::WAITING_CONNECTIONS_TIMEOUT));
                rx
            },
        };
        debug!(%task_id, "Read waiting on write");
        let recv_res = recv.await;
        _ = state.task_manager.remove(&task_id);
        match recv_res {
            Ok(s) => Ok(Body::from_stream(s).into_response()),
            Err(_) => {
                warn!(%task_id, "Socket connection expired");
                Err(StatusCode::GONE)
            },
        }
    } else {
        let sender = match state.waiting_connections.entry(task_id) {
            Entry::Occupied(e) => {
                match e.remove().0 {
                    ConnectionState::ReadHalfConnected(send_read) => send_read,
                    ConnectionState::WriteHalfConnected(_) => {
                        warn!("Sender connected twice");
                        return Err(StatusCode::INTERNAL_SERVER_ERROR);
                    },
                }
            },
            Entry::Vacant(empty) => {
                let (tx, rx) = oneshot::channel();
                empty.insert((ConnectionState::WriteHalfConnected(rx), Instant::now() + SocketState::WAITING_CONNECTIONS_TIMEOUT));
                tx
            },
        };
        let (tx, write_done) = oneshot::channel::<()>();
        let mut notify_write_done = Some(tx);
        let send_res = sender.send(stream::once(futures_util::future::ready(Ok(remaining.freeze()))).chain(body_stream).chain(stream::poll_fn(move |_| {
            _ = notify_write_done.take().unwrap().send(());
            std::task::Poll::Ready(None)
        })).boxed());
        let Ok(()) = send_res else {
            warn!(%task_id, "Failed to send socket body. Reciever dropped");
            return Err(StatusCode::GONE);
        };
        debug!("Write half send the stream to the read half");
        _ = write_done.await;
        debug!("Write half has written all its data");
        Err(StatusCode::OK)
    }
}

async fn read_header(s: &mut BodyDataStream) -> Result<(bool, Bytes, BytesMut), StatusCode> {
    async fn next(s: &mut BodyDataStream) -> Result<Option<Bytes>, StatusCode> {
        s.next().await.transpose().map_err(|e| {
            warn!(%e, "Failed to read init for sockets");
            StatusCode::BAD_GATEWAY
        })
    }
    let mut buf = BytesMut::new();
    #[derive(Debug)]
    enum ReadState {
        ReadingHeader,
        ReadingMessage { is_read: bool, len: usize },
    }
    let mut state = ReadState::ReadingHeader;
    while let Some(mut packet) = next(s).await?  {
        loop {
            match state {
                ReadState::ReadingHeader if buf.len() + packet.len() >= 5 => {
                    buf.put(packet.split_to(packet.len()));
                    debug_assert!(packet.is_empty());
                    let is_read = buf.split_to(1)[0] == 1;
                    let len = u32::from_be_bytes(buf.split_to(4).as_ref().try_into().unwrap());
                    state = ReadState::ReadingMessage { is_read, len: len as usize };
                    continue;
                },
                ReadState::ReadingMessage { is_read, len } if buf.len() + packet.len() >= len => {
                    buf.put(packet);
                    return Ok((is_read, buf.split_to(len).freeze(), buf))
                },
                _ => break,
            }
        }
        buf.put(packet);
    }
    warn!(?state, ?buf, "Failed to read header");
    Err(StatusCode::BAD_GATEWAY)
}
