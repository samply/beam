use std::sync::Arc;

use axum::{Router, Json, extract::State, routing::get, response::IntoResponse};
use hyper::{StatusCode, header};
use shared::{MsgSocketRequest, Encrypted, MsgSigned, HowLongToBlock, crypto_jwt::Authorized, MsgEmpty, Msg};
use tokio::sync::RwLock;


#[derive(Clone, Default)]
struct SocketState {
    socket_requests: Arc<RwLock<Vec<MsgSigned<MsgSocketRequest<Encrypted>>>>>,

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
    let HowLongToBlock { wait_time, wait_count } = block;
    let mut socket_reqs = state.socket_requests
        .read()
        .await
        .iter()
        .filter(|req| &req.msg.to == requester)
        .cloned()
        .collect();

    todo!("Add wait for missing tasks");

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
