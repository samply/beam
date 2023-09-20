use axum::{async_trait, body::Bytes, http::request, response::Response, Json};
use hyper::{client::HttpConnector, Client, Method, Request, StatusCode, Uri, header};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use beam_lib::AppOrProxyId;
use shared::{
    config,
    config_proxy::Config,
    config_shared::ConfigCrypto,
    crypto::GetCerts,
    errors::{CertificateInvalidReason, SamplyBeamError},
    http_client::SamplyHttpClient,
    EncryptedMessage, MsgEmpty,
};
use tracing::{debug, info, warn, error};

use crate::serve_tasks::sign_request;

pub(crate) struct GetCertsFromBroker {
    client: SamplyHttpClient,
    config: Config,
    crypto_conf: ConfigCrypto,
}

impl GetCertsFromBroker {
    async fn request(&self, path: &str) -> Result<Response<hyper::Body>, SamplyBeamError> {
        let uri = Uri::builder()
            .scheme(self.config.broker_uri.scheme().unwrap().to_owned())
            .authority(self.config.broker_uri.authority().unwrap().to_owned())
            .path_and_query(path)
            .build()?;

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

        let req = sign_request(body, parts, &self.config, Some(&self.crypto_conf))
            .await
            .map_err(|(_, msg)| SamplyBeamError::SignEncryptError(msg.into()))?;
        Ok(self.client.request(req).await?)
    }

    async fn query(&self, path: &str) -> Result<String, SamplyBeamError> {
        let mut req = self.request(path).await?;
        let resp = hyper::body::to_bytes(req.body_mut()).await?;
        let resp =
            String::from_utf8(resp.to_vec()).map_err(|e| SamplyBeamError::HttpParseError(e))?;
        match req.status() {
            StatusCode::NOT_FOUND => Ok(String::new()),
            StatusCode::NO_CONTENT => {
                debug!("Broker rejected to send us invalid certificate on path {path}");
                Err(CertificateInvalidReason::NotDisclosedByBroker.into())
            }
            StatusCode::OK => Ok(resp),
            x => Err(SamplyBeamError::VaultOtherError(format!(
                "Got code {x}, error message: {}",
                resp
            ))),
        }
    }

    async fn query_vec(&self, path: &str) -> Result<Vec<String>, SamplyBeamError> {
        let mut req = self.request(path).await?;
        let resp = hyper::body::to_bytes(req.body_mut()).await?;
        let json: Vec<String> = serde_json::from_slice(&resp).map_err(|e| {
            SamplyBeamError::VaultOtherError(format!("Unable to parse vault reply: {}", e))
        })?;
        match req.status() {
            StatusCode::NOT_FOUND => Ok(json),
            StatusCode::NO_CONTENT => {
                debug!("Broker rejected to send us invalid certificate on path {path}");
                Err(CertificateInvalidReason::NotDisclosedByBroker.into())
            }
            StatusCode::OK => Ok(json),
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

    async fn on_cert_expired(&self, expired_cert: shared::openssl::x509::X509) {
        // We can't use our own `ConfigCrypto` here as it is only an intermidate config for getting initial certs from the broker
        let own_cert = shared::crypto::get_own_crypto_material()
            .public
            .as_ref()
            .expect("Fatal error: Unable to read our own certificate.");
        let own_cert = &own_cert.cert;
        if expired_cert.serial_number() == own_cert.serial_number() {
            // TODO Tobias will find a smart solution ;)
            error!("Our own cert has just expired -- exiting.");
            std::process::exit(13);
        }
    }
}

pub(crate) fn build_cert_getter(
    config: Config,
    client: SamplyHttpClient,
    crypto_conf: ConfigCrypto,
) -> Result<GetCertsFromBroker, SamplyBeamError> {
    let client = client;
    let broker_url = config.broker_uri.clone();
    let _ = broker_url
        .scheme()
        .ok_or(SamplyBeamError::ConfigurationFailed(
            "Broker URL invalid.".into(),
        ))?;
    let _ = broker_url
        .authority()
        .ok_or(SamplyBeamError::ConfigurationFailed(
            "Broker URL invalid.".into(),
        ))?;
    // let broker_builder = Uri::builder()
    //     .scheme(scheme)
    //     .authority(authority);
    Ok(GetCertsFromBroker {
        client,
        config,
        crypto_conf,
    })
}
