use std::{sync::Arc, collections::{HashMap, HashSet}, ops::Deref, time::Duration};

use axum::{Router, Json, extract::{State, Path}, routing::get, response::{IntoResponse, Response}, headers::{ContentType, HeaderMapExt}};
use bytes::BufMut;
use hyper::{StatusCode, header, HeaderMap, http::HeaderValue, Body, Request, Method, upgrade::{OnUpgrade, self}};
use serde::{Serialize, Serializer, ser::SerializeSeq};
use shared::{MsgSocketRequest, Encrypted, MsgSigned, HowLongToBlock, crypto_jwt::Authorized, MsgEmpty, Msg, MsgId, HasWaitId, config::{CONFIG_SHARED, CONFIG_CENTRAL}, expire_map::LazyExpireMap, serde_helpers::DerefSerializer};
use tokio::sync::{RwLock, broadcast::{Sender, self}, oneshot};
use tracing::{debug, log::error, warn};

use crate::task_manager::{TaskManager, Task};


#[derive(Clone)]
struct SocketState {
    task_manager: Arc<TaskManager<MsgSocketRequest<Encrypted>>>,
    waiting_connections: Arc<LazyExpireMap<MsgId, oneshot::Sender<Request<Body>>>>
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
    mut req: Request<Body>
    // This Result is just an Either type. An error value does not mean something went wrong
) -> Result<Response, StatusCode> {
    // We have to do this reconstruction of the request as calling extract on the req to get the body will take ownership of the request
    let (mut parts, body) = req.into_parts();
    let body = hyper::body::to_bytes(body)
        .await
        .ok()
        .and_then(|data| String::from_utf8(data.to_vec()).ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
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
    req = Request::from_parts(parts, Body::empty());
    if req.extensions().get::<OnUpgrade>().is_none() {
        return Err(StatusCode::UPGRADE_REQUIRED);
    }

    if let Some(req_sender) = state.waiting_connections.remove(&task_id) {
        if let Err(_) = req_sender.send(req) {
            warn!("Error sending socket connection to tunnel. Receiver has been dropped");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    } else {
        let (tx, rx) = tokio::sync::oneshot::channel();
        state.waiting_connections.insert_for(SocketState::WAITING_CONNECTIONS_TIMEOUT, task_id, tx);
        let Ok(other_req) = rx.await else {
            debug!("Socket expired because nobody connected");
            return Err(StatusCode::GONE);
        };
        // We don't care if the task expired by now
        _ = state.task_manager.remove(&task_id);
        tokio::spawn(async move {
            let (mut socket1, mut socket2) = match tokio::try_join!(upgrade::on(req), upgrade::on(other_req)) {
                Ok(sockets) => sockets,
                Err(e) => {
                    warn!("Failed to upgrade requests to socket connections: {e}");
                    return;
                },
            };

            let result = tokio::io::copy_bidirectional(&mut socket1, &mut socket2).await;
            if let Err(e) = result {
                debug!("Relaying socket connection ended: {e}");
            }
        });
    }
    Err(StatusCode::SWITCHING_PROTOCOLS)
}
