use std::sync::Arc;

use axum::{Router, Json, extract::State, routing::get, response::IntoResponse};
use hyper::{StatusCode, header};
use shared::{MsgSocketRequest, Encrypted, MsgSigned, HowLongToBlock, crypto_jwt::Authorized, MsgEmpty, Msg, MsgId};
use tokio::sync::{RwLock, broadcast::{Sender, self}};

use crate::serve_tasks::wait_for_elements_task;


#[derive(Clone)]
struct SocketState {
    socket_requests: Arc<RwLock<Vec<MsgSigned<MsgSocketRequest<Encrypted>>>>>,
    /// TODO: Figure out a better way to do this at some point
    new_socket_request: Arc<Sender<MsgSigned<MsgSocketRequest<Encrypted>>>>,
    deleted_socket_request: Arc<Sender<MsgId>>,
}

impl Default for SocketState {
    fn default() -> Self {
        let (new_sender, _) = broadcast::channel(32);
        let (deleted_sender, _) = broadcast::channel(32);
        Self {
            socket_requests: Default::default(), 
            new_socket_request: Arc::new(new_sender),
            deleted_socket_request: Arc::new(deleted_sender)
        }
    }
}

pub(crate) fn router() -> Router {
    Router::new()
        .route("/v1/sockets", get(get_socket_requests).post(post_socket_request))
        .route("/v1/sockets/:id/results", todo!())
        .with_state(SocketState::default())
}


async fn get_socket_requests(
    block: HowLongToBlock,
    state: State<SocketState>,
    msg: MsgSigned<MsgEmpty>,
) -> Json<Vec<MsgSigned<MsgSocketRequest<Encrypted>>>> {
    let requester = msg.get_from();
    let filter = |req: &MsgSigned<MsgSocketRequest<Encrypted>>| &req.msg.to == requester;
    let mut socket_reqs = state.socket_requests
        .read()
        .await
        .iter()
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

    Json(socket_reqs)
}

async fn post_socket_request(
    state: State<SocketState>,
    msg: MsgSigned<MsgSocketRequest<Encrypted>>,
) -> impl IntoResponse {
    let msg_id = msg.msg.id.clone();
    state.socket_requests
        .write()
        .await
        .push(msg);

    todo!("inform subscribers");
    (
        StatusCode::CREATED,
        [(header::LOCATION, format!("/v1/tasks/{}", msg_id))]
    )
}
