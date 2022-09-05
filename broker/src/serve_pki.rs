// GET /v1/pki/*path

use std::{convert::Infallible, string::FromUtf8Error};

use axum::{Extension, http::Request, routing::{Route, get}, Router, response::{Response, IntoResponse}, Json, extract::{Query, Path}};
use hyper::{Client, Body, client::HttpConnector, StatusCode};
use hyper_tls::HttpsConnector;
use serde::{Serialize, Deserialize};
use shared::{config::CONFIG_CENTRAL, errors::SamplyBeamError};
use thiserror::Error;
use tracing::{error, info, debug};
use vaultrs::pki;

#[derive(Error, Debug)]
enum PkiError {
    #[error("Error in communication with PKI. {0}")]
    CommunicationWithVault(String),
    #[error("Error processing certificate: {0}")]
    OpenSslError(String),
    #[error("Unable to parse response: {0}")]
    ParseError(#[from] FromUtf8Error)
}

impl IntoResponse for PkiError {
    fn into_response(self) -> Response {
        let status = match self {
            PkiError::CommunicationWithVault(_)
                => StatusCode::BAD_GATEWAY,
            PkiError::OpenSslError(_) | PkiError::ParseError(_)
                => StatusCode::PRECONDITION_FAILED,
        };
        
        (status, self.to_string()).into_response()
    }
}

pub(crate) fn router() -> Router {
    Router::new()
        .route("/v1/pki/certs", get(get_certificate_list))
        .route("/v1/pki/certs/im-ca", get(get_im_cert))
        .route("/v1/pki/certs/by_serial/:serial", get(get_certificate_by_serial))
}

async fn get_certificate_by_serial(
    Path(serial): Path<String>
) -> Result<String, PkiError> {
    debug!("=> Asked for Serial {serial}");
    let cert = shared::crypto::get_cert_and_client_by_serial_as_pemstr(&serial).await
        .ok_or(PkiError::CommunicationWithVault(String::new()))?;
    let pem = cert.cert.to_pem()
        .map_err(|e| PkiError::OpenSslError(e.to_string()))?;
    Ok(String::from_utf8(pem)?)
}

async fn get_im_cert() -> Result<String, PkiError> {
    debug!("=> Asked for IM CA Cert");
    let cert = shared::crypto::get_im_cert().await
        .or(Err(PkiError::CommunicationWithVault(String::new())))?;
    Ok(cert)
}

async fn get_certificate_list() -> Result<Json<Vec<String>>,PkiError> {
    debug!("Asked for all certificates.");
    let list = shared::crypto::get_serial_list().await
        .map_err(|e| PkiError::CommunicationWithVault(e.to_string()))?;
    let json = Json(list);
    Ok(json)
} 