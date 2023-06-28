use http::{request, Request, header};
use serde::Serialize;

use crate::AddressingId;


pub trait BeamRequestBuilderExt {
    fn as_app(self, app: &AddressingId, key: &str) -> Self;
    // We do a generic B here to not require hyper as a dependency
    fn with_json<B: From<Vec<u8>>, T: Serialize>(self, json: &T) -> Result<Request<B>, http::Error>;
}

impl BeamRequestBuilderExt for request::Builder {
    fn as_app(self, app: &AddressingId, key: &str) -> Self {
        self.header(header::AUTHORIZATION, format!("ApiKey {app} {key}"))
    }

    fn with_json<B: From<Vec<u8>>, T: Serialize>(self, json: &T) -> Result<Request<B>, http::Error> {
        self.body(B::from(serde_json::to_vec(json).unwrap()))
    }
}
