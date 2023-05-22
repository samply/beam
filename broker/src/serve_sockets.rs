use std::{sync::Arc, collections::{HashMap, HashSet}, ops::Deref};

use axum::{Router, Json, extract::{State, Path}, routing::get, response::{IntoResponse, Response}};
use bytes::BufMut;
use hyper::{StatusCode, header, HeaderMap, http::HeaderValue, Body, Request, Method, upgrade::OnUpgrade};
use serde::{Serialize, Serializer, ser::SerializeSeq};
use shared::{MsgSocketRequest, Encrypted, MsgSigned, HowLongToBlock, crypto_jwt::Authorized, MsgEmpty, Msg, MsgId, HasWaitId, config::{CONFIG_SHARED, CONFIG_CENTRAL}};
use tokio::sync::{RwLock, broadcast::{Sender, self}, oneshot};
use tracing::{debug, log::error, warn};

use crate::{serve_tasks::wait_get_statuscode, task_manager::{TaskManager, Task}};


#[derive(Clone)]
struct SocketState {
    task_manager: Arc<TaskManager<MsgSocketRequest<Encrypted>>>,
    waiting_connections: Arc<RwLock<HashMap<MsgId, oneshot::Sender<Request<Body>>>>>
}

impl Default for SocketState {
    fn default() -> Self {
        Self {
            task_manager: Arc::new(TaskManager::new()),
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

// Look into making a PR for Dashmap that makes its smartpointers serialize with the serde feature
fn serialize_deref_iter<S: Serializer>(serializer: S, iter: impl Iterator<Item = impl Deref<Target = impl Serialize>>) -> Result<S::Ok, S::Error> {
    let mut seq_ser = serializer.serialize_seq(iter.size_hint().1).unwrap();
    for item in iter {
        seq_ser.serialize_element(item.deref())?;
    }
    seq_ser.end()
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

    let writer = bytes::BytesMut::new().writer(); 
    let mut serializer = serde_json::Serializer::new(writer);
    if let Err(e) = serialize_deref_iter(&mut serializer, socket_reqs) {
        warn!("Failed to serialize socket tasks: {e}");
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    Ok(Response::builder()
        .header(header::CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(Body::from(serializer.into_inner().into_inner().freeze()))
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

async fn connect_socket(
    state: State<SocketState>,
    task_id: MsgId,
    mut req: Request<Body>
) -> Response {
    // We have to do this reconstruction of the request as calling extract on the req to get the body will take ownership of the request
    let (mut parts, body) = req.into_parts();
    let body = String::from_utf8(hyper::body::to_bytes(body).await.unwrap().to_vec()).unwrap();
    let result = shared::crypto_jwt::verify_with_extended_header::<MsgEmpty>(&mut parts, &body).await;
    // TODO: Maybe check from
    let msg = match result {
        Ok(msg) => msg.msg,
        Err(e) => return e.into_response(),
    };
    req = Request::from_parts(parts, Body::empty());
    if req.extensions().get::<OnUpgrade>().is_none() {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    }

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
