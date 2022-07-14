use axum::{async_trait, extract::{FromRequest, Query, Path}, BoxError, http::StatusCode};

use crate::*;
use dataobjects::{MyUuid, HowLongToBlock, HowLongToBlockAsIntegers};

#[async_trait]
impl<B> FromRequest<B> for HowLongToBlock
where
B: axum::body::HttpBody + Send,
B::Data: Send,
B::Error: Into<BoxError>
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request(req: &mut axum::extract::RequestParts<B>) -> Result<Self, Self::Rejection> {
        match req.extract::<Query<HowLongToBlockAsIntegers>>().await {
            Ok(value) => {
                let poll_timeout = value.0.poll_timeout.map(Duration::from_millis);
                Ok(Self { poll_timeout, poll_count: value.0.poll_count })
            },
            Err(_) => Err((StatusCode::BAD_REQUEST, "For long-polling, please define &timeout=<millisecs> and &resultcount=<count>.")),
        }
    }
}

#[async_trait]
impl<B> FromRequest<B> for MyUuid
where
// these trait bounds are copied from `impl FromRequest for axum::Json`
B: axum::body::HttpBody + Send,
B::Data: Send,
B::Error: Into<BoxError>
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request(req: &mut axum::extract::RequestParts<B>) -> Result<Self, Self::Rejection> {
        match req.extract::<Path<Uuid>>().await {
            Ok(value) => Ok(Self(value.0)),
            Err(_) => Err((StatusCode::BAD_REQUEST, "Invalid ID supplied.")),
        }
    }
}
