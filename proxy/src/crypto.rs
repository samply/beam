use axum::{body::Bytes, http::{header, request, Method, Request, StatusCode, Uri}, response::Response, Json};
use beam_lib::AppOrProxyId;
use shared::{
    async_trait, config, config_proxy::Config, config_shared::ConfigCrypto, crypto::GetCerts, errors::{CertificateInvalidReason, SamplyBeamError}, http_client::SamplyHttpClient, reqwest, EncryptedMessage, MsgEmpty
};
use tracing::{debug, info, warn, error};

use crate::serve_tasks::sign_request;

pub(crate) struct GetCertsFromBroker {
    client: SamplyHttpClient,
    config: Config,
    crypto_conf: ConfigCrypto,
}

impl GetCertsFromBroker {
    async fn request(&self, path: &str) -> Result<reqwest::Response, SamplyBeamError> {
        let uri = Uri::builder()
            .scheme(self.config.broker_uri.scheme())
            .authority(self.config.broker_uri.authority())
            .path_and_query(path)
            .build()
            .expect("To build request successfully");

        let body = EncryptedMessage::MsgEmpty(MsgEmpty {
            from: AppOrProxyId::Proxy(self.config.proxy_id.clone()),
        });
        let (parts, body) = Request::builder()
            .method(Method::GET)
            .uri(&uri)
            .header(header::USER_AGENT, env!("SAMPLY_USER_AGENT"))
            .body(body)
            .expect("To build request successfully")
            .into_parts();

        let req = sign_request(body, parts, &self.config, &self.crypto_conf)
            .await
            .map_err(|(_, msg)| SamplyBeamError::SignEncryptError(msg.into()))?;
        Ok(self.client.execute(req).await?.into())
    }

    async fn query(&self, path: &str) -> Result<String, SamplyBeamError> {
        let req = self.request(path).await?;
        match req.status() {
            StatusCode::NOT_FOUND => Ok(String::new()),
            StatusCode::NO_CONTENT => {
                debug!("Broker rejected to send us invalid certificate on path {path}");
                Err(CertificateInvalidReason::NotDisclosedByBroker.into())
            }
            StatusCode::OK => Ok(req.text().await?),
            StatusCode::UNAUTHORIZED => Err(SamplyBeamError::BrokerAuthorizationFailed),
            x => Err(SamplyBeamError::VaultOtherError(format!(
                "Got code {x}, error message: {}",
                req.text().await?
            ))),
        }
    }

    async fn query_vec(&self, path: &str) -> Result<Vec<String>, SamplyBeamError> {
        let req = self.request(path).await?;
        match req.status() {
            StatusCode::NOT_FOUND | StatusCode::OK => {
                let resp = req.bytes().await?;
                serde_json::from_slice(&resp).map_err(|e| {
                    SamplyBeamError::VaultOtherError(format!("Unable to parse vault reply: {}", e))
                })
            },
            StatusCode::NO_CONTENT => {
                debug!("Broker rejected to send us invalid certificate on path {path}");
                Err(CertificateInvalidReason::NotDisclosedByBroker.into())
            }
            x => Err(SamplyBeamError::VaultOtherError(format!("Got code {x}"))),
        }
    }
}

#[async_trait]
impl GetCerts for GetCertsFromBroker {
    async fn certificate_list_via_network(&self) -> Result<Vec<String>, SamplyBeamError> {
        debug!("Retrieving cert list from Broker ...");
        self.query_vec("/v1/pki/certs").await
    }

    async fn certificate_by_serial_as_pem(&self, serial: &str) -> Result<String, SamplyBeamError> {
        debug!("Retrieving certificate with serial {serial} ...");
        self.query(&format!("/v1/pki/certs/by_serial/{}", serial))
            .await
    }

    async fn im_certificate_as_pem(&self) -> Result<String, SamplyBeamError> {
        debug!("Retrieving intermediate CA certificate ...");
        self.query("/v1/pki/certs/im-ca").await
    }
}

pub(crate) fn build_cert_getter(
    config: Config,
    client: SamplyHttpClient,
    crypto_conf: ConfigCrypto,
) -> Result<GetCertsFromBroker, SamplyBeamError> {
    let client = client;
    Ok(GetCertsFromBroker {
        client,
        config,
        crypto_conf,
    })
}
