use axum::{http::StatusCode, routing::get, Router};

pub(crate) fn router() -> Router {
    Router::new().route("/v1/health", get(handler_health))
}

async fn handler_health() -> StatusCode {
    StatusCode::OK
}
