use axum::{Router, routing::get};
use shared::{http_client::SamplyHttpClient, config};

use crate::serve_tasks::{handler_task, TasksState};


pub(crate) fn router(client: SamplyHttpClient) -> Router {
    let config = config::CONFIG_PROXY.clone();
    let state = TasksState {
        client: client.clone(),
        config,
    };
    Router::new()
        .route("/v1/sockets", get(handler_task).post(handler_task))
        .route("/v1/sockets/:id/results", get(handler_task).put(handler_task))
        .with_state(state)
}
