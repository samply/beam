use std::sync::Arc;

use axum::{Router, Json, extract::State, routing::get, response::IntoResponse};
use shared::{MsgSocketRequest, Encrypted, MsgSigned, HowLongToBlock, crypto_jwt::Authorized, MsgEmpty};
use tokio::sync::RwLock;


#[derive(Clone, Default)]
struct SocketState {
    socket_requests: Arc<RwLock<Vec<MsgSigned<MsgSocketRequest<Encrypted>>>>>,

}

pub(crate) fn router() -> Router {
    Router::new()
        .route("/v1/sockets", get(get_socket_requests).post(post_socket_request))
        .route("/v1/sockets/results", todo!())
        .with_state(SocketState::default())
}


async fn get_socket_requests(
    block: HowLongToBlock,
    state: State<SocketState>,
    msg: MsgSigned<MsgEmpty>,
) -> Json<Vec<MsgSigned<MsgSocketRequest<Encrypted>>>> {
    todo!()
}

async fn post_socket_request() -> impl IntoResponse {
    todo!()
}
