#[cfg(feature = "backend-warp")]
use warp::{hyper::StatusCode, reject::Reject};

#[cfg(feature = "backend-axum")]
use axum::http::StatusCode;

#[derive(Debug)]
pub struct WebError {
    pub reason: String,
    pub code: StatusCode,
}

#[cfg(feature = "backend-warp")]
impl Reject for WebError {}
