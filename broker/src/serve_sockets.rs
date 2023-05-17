use std::{sync::Arc, collections::{HashMap, HashSet}, ops::Deref};

use axum::{Router, Json, extract::{State, Path}, routing::get, response::{IntoResponse, Response}};
use bytes::BufMut;
use hyper::{StatusCode, header, HeaderMap, http::HeaderValue, Body, Request, Method};
use shared::{MsgSocketRequest, Encrypted, MsgSigned, HowLongToBlock, crypto_jwt::Authorized, MsgEmpty, Msg, MsgId, MsgSocketResult, HasWaitId, config::{CONFIG_SHARED, CONFIG_CENTRAL}};
use tokio::sync::{RwLock, broadcast::{Sender, self}, oneshot};
use tracing::{debug, log::error, warn};

use crate::{serve_tasks::wait_get_statuscode, task_manager::{TaskManager, Task}};


#[derive(Clone)]
struct SocketState {
    task_manager: Arc<TaskManager<MsgSocketRequest<Encrypted>>>,
    allowed_tokens: Arc<RwLock<HashSet<String>>>,
    waiting_connections: Arc<RwLock<HashMap<MsgId, oneshot::Sender<Request<Body>>>>>
}

impl Default for SocketState {
    fn default() -> Self {
        Self {
            task_manager: Arc::new(TaskManager::new()),
            allowed_tokens: Default::default(),
            waiting_connections: Default::default()
        }
    }
}

pub(crate) fn router() -> Router {
    Router::new()
        .route("/v1/sockets", get(get_socket_requests).post(post_socket_request))
        .route("/v1/sockets/:id", get(connect_socket))
        .route("/v1/sockets/:id/results", get(get_socket_result).put(put_socket_result))
        .with_state(SocketState::default())
}


async fn get_socket_requests(
    block: HowLongToBlock,
    state: State<SocketState>,
    msg: MsgSigned<MsgEmpty>,
) -> Result<Response, StatusCode> {
    if block.wait_count.is_none() && block.wait_time.is_none() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let requester = msg.get_from();
    let filter = |req: &MsgSocketRequest<Encrypted>| req.to.contains(requester);

    let socket_reqs = state.task_manager.wait_for_tasks(&block, filter).await?;

    // Make a PR to DashMap that enables the Locks to be Serialize with the serde feature
    let mut writer = bytes::BytesMut::new().writer(); 
    for task in socket_reqs {
        serde_json::to_writer(&mut writer, task.deref()).map_err(|e| {
            warn!("Error serializing task: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    }

    Ok(Response::builder()
        .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(Body::from(writer.into_inner().freeze()))
        .expect("This is a proper Response")
        .into_response()
    )
}

async fn post_socket_request(
    state: State<SocketState>,
    msg: MsgSigned<MsgSocketRequest<Encrypted>>,
) -> Result<impl IntoResponse, StatusCode> {
    let msg_id = msg.wait_id();
    state.task_manager.post_task(msg)?;

    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, format!("/v1/sockets/{}/results", msg_id))]
    ))
}

async fn put_socket_result(
    state: State<SocketState>,
    task_id: MsgId,
    result: MsgSigned<MsgSocketResult>
) -> Result<(StatusCode, impl IntoResponse), StatusCode> {
    if task_id != result.msg.task {
        return Err(StatusCode::BAD_REQUEST);
    };
    
    let token = result.msg.token.clone();
    state.task_manager.put_result(&task_id, result)?;
    state.allowed_tokens.write().await.insert(token);
    
    Ok((StatusCode::CREATED, [(header::LOCATION, format!("socks5://{}:{}", CONFIG_SHARED.broker_domain, CONFIG_CENTRAL.socket_port))]))
}

async fn get_socket_result(
    state: State<SocketState>,
    mut block: HowLongToBlock,
    task_id: MsgId,
    msg: MsgSigned<MsgEmpty>
) -> Result<Response, StatusCode> {
    let body = {
        let task = state.task_manager.get(&task_id)?;
        if task.get_from() != msg.get_from() {
            return Err(StatusCode::UNAUTHORIZED);
        }
        // If we already have a result return it
        if let Some(result) = task.msg.result.first() {
            Some(serde_json::to_vec(result).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?)
        } else {
            None
        }
    };

    let body = if let Some(result) = body {
        result
    } else {
        block.wait_count = Some(1);
        let task = state.task_manager.wait_for_results(
            &task_id,
            &block,
            |m| m.msg.to.contains(msg.get_from()),
        ).await?;
        let Some(result) = task.msg.get_results().first() else {
            return Err(StatusCode::NO_CONTENT)
        };
        serde_json::to_vec(result).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    let location = HeaderValue::from_str(&format!("socks5://{}:{}", CONFIG_SHARED.broker_domain, CONFIG_CENTRAL.socket_port)).expect("Broker domain cotains invalid chars");
    Ok(Response::builder()
        .header(header::LOCATION, location)
        .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(Body::from(body))
        .expect("This should be a valid body")
        .into_response()
    )
}

// How to auth here?
async fn connect_socket(
    state: State<SocketState>,
    task_id: MsgId,
    req: Request<Body>
) -> Response {
    let mut waiting_cons = state.waiting_connections.write().await;
    if let Some(req_sender) = waiting_cons.remove(&task_id) {
        req_sender.send(req).unwrap();
    } else {
        let (tx, rx) = tokio::sync::oneshot::channel();
        waiting_cons.insert(task_id, tx);
        drop(waiting_cons);
        let other_req = rx.await.unwrap();
        tokio::spawn(async move {
            let mut c1 = hyper::upgrade::on(req).await.unwrap();
            let mut c2 = hyper::upgrade::on(other_req).await.unwrap();

            let result = tokio::io::copy_bidirectional(&mut c1, &mut c2).await;
            if let Err(e) = result {
                warn!("Error relaying socket connect: {e}");
            }
        });
    }
    StatusCode::SWITCHING_PROTOCOLS.into_response()
}
