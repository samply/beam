use std::{sync::Arc, collections::HashMap};

use axum::{Router, Json, extract::{State, Path}, routing::get, response::{IntoResponse, Response}};
use hyper::{StatusCode, header, HeaderMap, http::HeaderValue};
use shared::{MsgSocketRequest, Encrypted, MsgSigned, HowLongToBlock, crypto_jwt::Authorized, MsgEmpty, Msg, MsgId, MsgSocketResult, HasWaitId, config::{CONFIG_SHARED, CONFIG_CENTRAL}};
use tokio::sync::{RwLock, broadcast::{Sender, self}, oneshot};
use tracing::{debug, log::error};

use crate::{serve_tasks::{wait_for_elements_task, wait_get_statuscode, wait_for_results_for_task}, socks::ALLOWED_TOKENS};


#[derive(Clone)]
struct SocketState {
    /// TODO: Expire watcher
    socket_requests: Arc<RwLock<HashMap<MsgId, MsgSigned<MsgSocketRequest<Encrypted>>>>>,
    /// TODO: Figure out a better way to do this at some point
    new_socket_request: Arc<Sender<MsgSigned<MsgSocketRequest<Encrypted>>>>,
    deleted_socket_request: Arc<Sender<MsgId>>,
    new_result_tx: Arc<RwLock<HashMap<MsgId, Sender<MsgSigned<MsgSocketResult>>>>>,
}

impl Default for SocketState {
    fn default() -> Self {
        let (new_sender, _) = broadcast::channel(32);
        let (deleted_sender, _) = broadcast::channel(32);
        Self {
            socket_requests: Default::default(), 
            new_socket_request: Arc::new(new_sender),
            deleted_socket_request: Arc::new(deleted_sender),
            new_result_tx: Default::default()
        }
    }
}

pub(crate) fn router() -> Router {
    Router::new()
        .route("/v1/sockets", get(get_socket_requests).post(post_socket_request))
        .route("/v1/sockets/:id/results", get(get_socket_result).put(put_socket_result))
        .with_state(SocketState::default())
}


async fn get_socket_requests(
    block: HowLongToBlock,
    state: State<SocketState>,
    msg: MsgSigned<MsgEmpty>,
) -> (StatusCode, Json<Vec<MsgSigned<MsgSocketRequest<Encrypted>>>>) {
    let requester = msg.get_from();
    let filter = |req: &MsgSigned<MsgSocketRequest<Encrypted>>| &req.msg.to == requester;
    let mut socket_reqs = state.socket_requests
        .read()
        .await
        .values()
        .filter(|m| filter(*m))
        .cloned()
        .collect();

    wait_for_elements_task(
        &mut socket_reqs,
        &block,
        state.new_socket_request.subscribe(),
        filter,
        state.deleted_socket_request.subscribe()
    ).await;

    (wait_get_statuscode(&socket_reqs, &block), Json(socket_reqs))
}

async fn post_socket_request(
    state: State<SocketState>,
    msg: MsgSigned<MsgSocketRequest<Encrypted>>,
) -> Result<impl IntoResponse, impl IntoResponse> {
    let msg_id = msg.wait_id();
    {
        let mut task_lock = state.socket_requests.write().await;
        if task_lock.contains_key(&msg_id) {
            return Err((StatusCode::CONFLICT, format!("Msg Id {msg_id} is already taken.")));
        }
        task_lock.insert(msg_id.clone(), msg.clone());
    }

    if let Err(e) = state.new_socket_request.send(msg.clone()) {
        debug!("Unable to send notification: {}. Ignoring since probably noone is currently waiting for socket tasks.", e);
    }
    let (tx, _) = broadcast::channel(1);
    state.new_result_tx.write().await.insert(msg_id, tx);

    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, format!("/v1/sockets/{}/results", msg_id))]
    ))
}

async fn put_socket_result(
    state: State<SocketState>,
    task_id: MsgId,
    msg: MsgSigned<MsgSocketResult>
) -> Result<(StatusCode, impl IntoResponse), (StatusCode, &'static str)> {
    if task_id != msg.msg.task {
        return Err((
            StatusCode::BAD_REQUEST,
            "Task IDs supplied in path and payload do not match.",
        ));
    };
    {
        let socket_req_map = &mut state.socket_requests.write().await;
        let Some(task) = socket_req_map.get_mut(&msg.msg.task) else {
            return Err((StatusCode::NOT_FOUND, "Socket task not found"));
        };
        if msg.get_from() != &task.msg.to {
            return Err((StatusCode::UNAUTHORIZED, "Your result is not requested for this task."));
        }
        task.msg.result = Some(msg.clone());
        ALLOWED_TOKENS.write().await.insert(msg.msg.token.clone());
    }
    {
        let result_sender_map = &state.new_result_tx.read().await;
        let Some(tx) = result_sender_map.get(&msg.msg.task) else {
            error!("Found coresponding task but no sender was registerd for task {}.", msg.msg.task);
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "See broker logs"));
        };
        if let Err(e) = tx.send(msg) {
            debug!("Unable to send notification: {}. Ignoring since probably noone is currently waiting for tasks.", e);
        };
    }
    Ok((StatusCode::CREATED, [(header::LOCATION, format!("socks5://{}:{}", CONFIG_SHARED.broker_domain, CONFIG_CENTRAL.socket_port))]))
}

async fn get_socket_result(
    state: State<SocketState>,
    mut block: HowLongToBlock,
    task_id: MsgId,
    msg: MsgSigned<MsgEmpty>
) -> Result<Response, (StatusCode, &'static str)> {
    let result = {
        let task_map = state.socket_requests.read().await;
        let Some(task) = task_map.get(&task_id) else {
            return Err((StatusCode::NOT_FOUND, "Task not found"));
        };
        if task.get_from() != msg.get_from() {
            return Err((StatusCode::UNAUTHORIZED, "Not your task."));
        }
        task.msg.result.clone()
    };

    let result = if let Some(result) = result {
        result
    } else {
        block.wait_count = Some(1);
        let mut result = Vec::with_capacity(1);
        let Some(new_result_rx) = state.new_result_tx.read().await.get(&task_id).map(Sender::subscribe) else {
            error!("Failed to find result reciever for existing task");
            return Err((StatusCode::INTERNAL_SERVER_ERROR, "Internal server error"));
        };
        wait_for_results_for_task(
            &mut result,
            &block, 
            new_result_rx,
            |m| &m.msg.to == msg.get_from(),
            state.deleted_socket_request.subscribe(), 
            &task_id
        ).await;
        let Some(result) = result.pop() else {
            return Err((StatusCode::NO_CONTENT, "Task has no result yet"))
        };
        result
    };
    let mut res = Json(result).into_response();
    let location = HeaderValue::from_str(&format!("socks5://{}:{}", CONFIG_SHARED.broker_domain, CONFIG_CENTRAL.socket_port)).expect("Broker domain cotains invalid chars");
    res.headers_mut().append(header::LOCATION, location);
    Ok(res)
}
