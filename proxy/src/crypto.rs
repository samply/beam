use axum::{async_trait, Json, body::Bytes, response::Response, http::request};
use hyper::{Client, client::HttpConnector, Uri, StatusCode, Request, Method};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use shared::{crypto::GetCerts, errors::{SamplyBeamError, CertificateInvalidReason}, config, config_proxy::Config, http_client::SamplyHttpClient, MsgEmpty, beam_id::AppOrProxyId, config_shared::ConfigCrypto};
use tracing::{debug, warn, info};

use crate::serve_tasks::sign_request;

pub(crate) struct GetCertsFromBroker {
    client: SamplyHttpClient,
    config: Config,
    crypto_conf: ConfigCrypto
}

impl GetCertsFromBroker {
    async fn request(&self, path: &str) -> Result<Response<hyper::Body>, SamplyBeamError> {
        let uri = Uri::builder()
            .scheme(self.config.broker_uri.scheme().unwrap().to_owned())
            .authority(self.config.broker_uri.authority().unwrap().to_owned())
            .path_and_query(path)
            .build()?;

        let body = serde_json::to_value(MsgEmpty { from: AppOrProxyId::ProxyId(self.config.proxy_id.clone()) }).expect("Emptymsg faild to serialize");
        let (parts, body) = Request::builder().method(Method::GET).uri(&uri).body(body).expect("To build request successfully").into_parts();
        let req = sign_request(body, parts, &self.config, &uri, AppOrProxyId::ProxyId(self.config.proxy_id.clone()), Some(&self.crypto_conf))
            .await
            .map_err(|(_, msg)| {
                SamplyBeamError::SignEncryptError(msg.into())
            })?;
        Ok(self.client.request(req).await?)
    }

    async fn query(&self, path: &str) -> Result<String,SamplyBeamError> {
        let mut req = self.request(path).await?;
        let resp = hyper::body::to_bytes(req.body_mut()).await?;
        let resp = String::from_utf8(resp.to_vec())
            .map_err(|e| SamplyBeamError::HttpParseError(e))?;
        match req.status() {
            StatusCode::NOT_FOUND => Ok(String::new()),
            StatusCode::NO_CONTENT => {
                debug!("Broker rejected to send us invalid certificate on path {path}");
                Err(CertificateInvalidReason::NotDisclosedByBroker.into())
            },
            StatusCode::OK => Ok(resp),
            x => Err(SamplyBeamError::VaultOtherError(format!("Got code {x}, error message: {}", resp)))
        }
    }

    async fn query_vec(&self, path: &str) -> Result<Vec<String>,SamplyBeamError> {
        let mut req = self.request(path).await?;
        let resp = hyper::body::to_bytes(req.body_mut()).await?;
        let json: Vec<String> = serde_json::from_slice(&resp)
            .map_err(|e| SamplyBeamError::VaultOtherError(format!("Unable to parse vault reply: {}", e)))?;
        match req.status() {
            StatusCode::NOT_FOUND => Ok(json),
            StatusCode::NO_CONTENT => {
                debug!("Broker rejected to send us invalid certificate on path {path}");
                Err(CertificateInvalidReason::NotDisclosedByBroker.into())
            },
            StatusCode::OK => Ok(json),
            x => Err(SamplyBeamError::VaultOtherError(format!("Got code {x}")))
        }
    }
}

#[async_trait]
impl GetCerts for GetCertsFromBroker {
    async fn certificate_list(&self) -> Result<Vec<String>,SamplyBeamError> {
        self.query_vec("/v1/pki/certs").await
    }

    async fn certificate_by_serial_as_pem(&self, serial: &str) -> Result<String,SamplyBeamError> {
        debug!("Retrieving certificate with serial {serial} ...");
        self.query(&format!("/v1/pki/certs/by_serial/{}", serial)).await
    }

    async fn im_certificate_as_pem(&self) -> Result<String,SamplyBeamError> {
        debug!("Retrieving im ca certificate ...");
        self.query("/v1/pki/certs/im-ca").await
    }
}

pub(crate) fn build_cert_getter(
    config: Config, 
    client: SamplyHttpClient,
    crypto_conf: ConfigCrypto
) -> Result<GetCertsFromBroker,SamplyBeamError> {
    let client = client;
    let broker_url = config.broker_uri.clone();
    let _ = broker_url.scheme().ok_or(SamplyBeamError::ConfigurationFailed("Broker URL invalid.".into()))?;
    let _ = broker_url.authority().ok_or(SamplyBeamError::ConfigurationFailed("Broker URL invalid.".into()))?;
    // let broker_builder = Uri::builder()
    //     .scheme(scheme)
    //     .authority(authority);
    Ok(GetCertsFromBroker { client, config, crypto_conf })
}
