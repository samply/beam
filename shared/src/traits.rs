use axum::{async_trait, extract::{FromRequest, Query, Path, FromRequestParts, self}, BoxError, http::StatusCode, RequestPartsExt};
use http::request::Parts;

use crate::*;

#[derive(Deserialize)]
struct HowLongToBlockAsIntegers {
    wait_time: Option<u64>,
    wait_count: Option<u16>,
}

#[async_trait]
impl<S> FromRequestParts<S> for HowLongToBlock
where
S: Send + Sync
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(req: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        match req.extract::<Query<HowLongToBlockAsIntegers>>().await {
            Ok(value) => {
                let wait_time = value.0.wait_time.map(Duration::from_millis);
                Ok(Self { wait_time, wait_count: value.0.wait_count })
            },
            Err(_) => Err((StatusCode::BAD_REQUEST, "For long-polling, please define &wait_time=<millisecs> and &wait_count=<count>.")),
        }
    }
}

#[async_trait]
impl<S> FromRequestParts<S> for MyUuid
where
S: Send + Sync
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(req: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match req.extract::<Path<Uuid>>().await {
            Ok(value) => Ok(Self(value.0)),
            Err(_) => Err((StatusCode::BAD_REQUEST, "Invalid ID supplied.")),
        }
    }
}