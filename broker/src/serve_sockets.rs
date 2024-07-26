use std::{sync::Arc, collections::{HashMap, HashSet}, ops::Deref, time::Duration};

use axum::{extract::{Path, Request, State}, http::{header, request::Parts, StatusCode}, response::{IntoResponse, Response}, routing::get, RequestExt, Router};
use bytes::BufMut;
use hyper_util::rt::TokioIo;
use serde::{Serialize, Serializer, ser::SerializeSeq};
use shared::{config::{CONFIG_CENTRAL, CONFIG_SHARED}, crypto_jwt::Authorized, expire_map::LazyExpireMap, serde_helpers::DerefSerializer, Encrypted, HasWaitId, HowLongToBlock, Msg, MsgEmpty, MsgId, MsgSigned, MsgSocketRequest};
use tokio::sync::{RwLock, broadcast::{Sender, self}, oneshot};
use tracing::{debug, log::error, warn};

use crate::task_manager::{TaskManager, Task};


#[derive(Clone)]
struct SocketState {
    task_manager: Arc<TaskManager<MsgSocketRequest<Encrypted>>>,
    waiting_connections: Arc<LazyExpireMap<MsgId, oneshot::Sender<hyper::upgrade::OnUpgrade>>>
}

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

async fn connect_socket(
    state: State<SocketState>,
    Path(task_id): Path<MsgId>,
    mut parts: Parts,
    body: String,
    // This Result is just an Either type. An error value does not mean something went wrong
) -> Result<Response, StatusCode> {
    let result = shared::crypto_jwt::verify_with_extended_header::<MsgEmpty>(&mut parts, &body).await;
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

    let Some(conn) = parts.extensions.remove::<hyper::upgrade::OnUpgrade>() else {
        return Err(StatusCode::UPGRADE_REQUIRED);
    };

    if let Some(req_sender) = state.waiting_connections.remove(&task_id) {
        if let Err(_) = req_sender.send(conn) {
            warn!("Error sending socket connection to tunnel. Receiver has been dropped");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    } else {
        let (tx, rx) = tokio::sync::oneshot::channel();
        state.waiting_connections.insert_for(SocketState::WAITING_CONNECTIONS_TIMEOUT, task_id, tx);
        let Ok(other_con) = rx.await else {
            debug!("Socket expired because nobody connected");
            return Err(StatusCode::GONE);
        };
        // We don't care if the task expired by now
        _ = state.task_manager.remove(&task_id);
        tokio::spawn(async move {
            let (socket1, socket2) = match tokio::try_join!(conn, other_con) {
                Ok(sockets) => sockets,
                Err(e) => {
                    warn!("Failed to upgrade requests to socket connections: {e}");
                    return;
                },
            };

            let result = tokio::io::copy_bidirectional(&mut TokioIo::new(socket1), &mut TokioIo::new(socket2)).await;
            if let Err(e) = result {
                debug!("Relaying socket connection ended: {e}");
            }
        });
    }
    Err(StatusCode::SWITCHING_PROTOCOLS)
}
