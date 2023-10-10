// GET /v1/pki/*path

use std::{convert::Infallible, net::SocketAddr, string::FromUtf8Error};

use axum::{
    extract::{ConnectInfo, Path, Query},
    http::Request,
    response::{IntoResponse, Response},
    routing::{get, Route},
    Extension, Json, Router,
};
use hyper::{client::HttpConnector, Body, Client, StatusCode};
use hyper_tls::HttpsConnector;
use serde::{Deserialize, Serialize};
use shared::{
    config::CONFIG_CENTRAL,
    crypto_jwt::Authorized,
    errors::{CertificateInvalidReason, SamplyBeamError},
};
use thiserror::Error;
use tracing::{debug, error, info, log::warn};

#[derive(Error, Debug)]
enum PkiError {
    #[error("Broker has trouble communicating with PKI. {0}")]
    CommunicationWithVault(String),
    #[error("Error processing certificate: {0}")]
    OpenSslError(String),
    #[error("Unable to parse response: {0}")]
    ParseError(#[from] FromUtf8Error),
    #[error("Certificate is present but invalid, please see broker logs.")]
    CertificateError,
}

impl IntoResponse for PkiError {
    fn into_response(self) -> Response {
        let status = match self {
            PkiError::CommunicationWithVault(_) => StatusCode::BAD_GATEWAY,
            PkiError::OpenSslError(_) | PkiError::ParseError(_) => StatusCode::PRECONDITION_FAILED,
            PkiError::CertificateError => StatusCode::NO_CONTENT,
        };

        (status, self.to_string()).into_response()
    }
}

pub(crate) fn router() -> Router {
    Router::new()
        .route("/v1/pki/certs", get(get_certificate_list))
        .route("/v1/pki/certs/im-ca", get(get_im_cert))
        .route(
            "/v1/pki/certs/by_serial/:serial",
            get(get_certificate_by_serial),
        )
}

#[tracing::instrument(name = "/v1/pki/certs/by_serial/:serial")]
async fn get_certificate_by_serial(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(serial): Path<String>,
    _: Authorized,
) -> Result<String, PkiError> {
    debug!("=> Asked for cert with serial {serial} by {addr}");
    let cert = match tokio::time::timeout(
        std::time::Duration::new(10, 0),
        shared::crypto::get_cert_and_client_by_serial_as_pemstr(&serial),
    )
    .await
    {
        Ok(certificate) => certificate.ok_or_else(|| {
            let err = format!("Cannot retrieve certificate for serial {serial}");
            warn!("{err}");
            PkiError::CommunicationWithVault(err)
        }),
        Err(e) => {
            let err = format!("Request for certificate with serial {serial} timed out: {e}");
            error!("{err}");
            Err(PkiError::CommunicationWithVault(err))
        }
    }?;
    let pem = cert
        .map_err(|_err| PkiError::CertificateError)?
        .cert
        .to_pem()
        .map_err(|e| PkiError::OpenSslError(e.to_string()))?;
    debug!("<= Returning requested cert with serial {serial}");
    Ok(String::from_utf8(pem)?)
}

#[tracing::instrument(name = "/v1/pki/certs/im-ca")]
async fn get_im_cert(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    _: Authorized,
) -> Result<String, PkiError> {
    debug!("=> Asked for IM CA Cert by {addr}");
    let cert = shared::crypto::get_im_cert()
        .await
        .or(Err(PkiError::CommunicationWithVault(String::new())))?;
    Ok(cert)
}

#[tracing::instrument(name = "/v1/pki/certs")]
async fn get_certificate_list(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    _: Authorized,
) -> Result<Json<Vec<String>>, PkiError> {
    debug!("Asked for all certificates by {addr}");
    let list = shared::crypto::get_serial_list().await;
    let json = Json(list);
    Ok(json)
}
