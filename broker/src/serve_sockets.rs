use std::{sync::Arc, collections::HashMap, time::Duration};

use axum::{Router, Json, extract::{State, Path}, routing::get, response::{IntoResponse, Response}};
use futures::StreamExt;
use hyper::{StatusCode, header, Body, Request, upgrade::{OnUpgrade, self}};
use serde::Serialize;
use shared::{MsgSocketRequest, Encrypted, MsgSigned, HowLongToBlock, MsgEmpty, Msg, MsgId};
use tokio::sync::{oneshot, Mutex};
use tracing::{debug, warn};

use crate::task_manager::TaskManager;

#[derive(Clone)]
struct SocketState {
    task_manager: Arc<TaskManager<MsgSocketRequest<Encrypted>>>,
    waiting_connections: Arc<Mutex<HashMap<MsgId, oneshot::Sender<Request<Body>>>>>
}

impl SocketState {
    const TTL: Duration = Duration::from_secs(60);
}

impl Default for SocketState {
    fn default() -> Self {
        Self {
            task_manager: TaskManager::new(),
            waiting_connections: Default::default()
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
) -> (StatusCode, Json<impl Serialize>) {
    if block.wait_count.is_none() && block.wait_time.is_none() {
        block.wait_count = Some(1);
    }
    let requester = msg.msg.from;
    let socket_tasks: Vec<_> = state.task_manager
        .stream_tasks(block, |t| std::future::ready(t.to.contains(&requester)))
        .collect()
        .await;
    let status = if block.wait_count.is_some_and(|wc| socket_tasks.len() < wc.into()) {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    (status, Json(socket_tasks))
}

async fn post_socket_request(
    state: State<SocketState>,
    msg: MsgSigned<MsgSocketRequest<Encrypted>>,
) -> Result<impl IntoResponse, StatusCode> {
    let msg_id = msg.msg.id;
    state.task_manager.post_task(msg).await?;

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
        let task = state.task_manager.get(task_id).await?;
        // Allowed to connect are the issuer of the task and the recipient
        if !(task.get_from() == &msg.from || task.get_to().contains(&msg.from)) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    req = Request::from_parts(parts, Body::empty());
    if req.extensions().get::<OnUpgrade>().is_none() {
        return Err(StatusCode::UPGRADE_REQUIRED);
    }

    let connection_channel = state.waiting_connections.lock().await.remove(&task_id);
    if let Some(req_sender) = connection_channel {
        if let Err(_) = req_sender.send(req) {
            warn!("Error sending socket connection to tunnel. Receiver has been dropped");
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    } else {
        let (tx, rx) = oneshot::channel();
        state.waiting_connections.lock().await.insert(task_id, tx);
        let other_req = tokio::select! {
            other_req = rx => {
                other_req.map_err(|_| {
                    warn!("Socket expired because the sender was dropped");
                    StatusCode::GONE
                })?
            }
            _ = tokio::time::sleep(SocketState::TTL) => {
                state.waiting_connections.lock().await.remove(&task_id);
                debug!("Socket expired because nobody connected after {}s", SocketState::TTL.as_secs());
                return Err(StatusCode::GONE)
            }
        };
        state.task_manager.remove(task_id).await;
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
