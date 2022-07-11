use axum::{http::StatusCode, routing::get, Router};

pub(crate) fn router() -> Router {
    Router::new()
        .route("/v1/health", get(handler))
}

// GET /v1/health
async fn handler() -> StatusCode {
    StatusCode::OK
}