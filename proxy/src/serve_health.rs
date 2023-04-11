use axum::{routing::get, Router};
use hyper::StatusCode;

pub(crate) fn router() -> Router {
    Router::new().route("/v1/health", get(handler_health))
}

async fn handler_health() -> StatusCode {
    StatusCode::OK
}
