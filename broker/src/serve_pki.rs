// GET /v1/pki/*path

use std::convert::Infallible;

use axum::{Extension, http::Request, routing::{Route, get}, Router, response::{Response, IntoResponse}, Json};
use hyper::{Client, Body, client::HttpConnector, StatusCode};
use hyper_tls::HttpsConnector;
use serde::Serialize;
use shared::config::CONFIG_CENTRAL;
use thiserror::Error;

#[derive(Error, Debug)]
enum PkiError {
    #[error("Some unused error")]
    Error1,
    #[error("Another unused error")]
    Error2,
}

impl IntoResponse for PkiError {
    fn into_response(self) -> Response {
        match self {
            PkiError::Error1 
          | PkiError::Error2 => (
                StatusCode::INTERNAL_SERVER_ERROR,
                self.to_string()
        )}.into_response()
    }
}

#[derive(Serialize,Clone)]
struct PkiInfo {
    url: String,
    realm: String,
}

pub(crate) fn router() -> Router {
    let pki_info = PkiInfo {
        url: CONFIG_CENTRAL.pki_address.to_string(),
        realm: CONFIG_CENTRAL.pki_realm.clone(),
    };

    Router::new()
        .route("/v1/pki/info", get(info))
        .layer(Extension(pki_info))
}

async fn info(
    // Extension(client): Extension<Client<HttpsConnector<HttpConnector>>>,
    // Extension(config): Extension<config_proxy::Config>,
    // req: Request<Body>,
    Extension(pki_info): Extension<PkiInfo>
) -> Result<Json<PkiInfo>, PkiError> {
    Ok(Json(pki_info))
}