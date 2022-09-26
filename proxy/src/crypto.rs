use axum::{async_trait, Json};
use hyper::{Client, client::HttpConnector, Uri, StatusCode};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use shared::{crypto::GetCerts, errors::SamplyBeamError, config, config_proxy::Config};
use tracing::debug;

pub(crate) struct GetCertsFromBroker {
    client: Client<ProxyConnector<HttpsConnector<HttpConnector>>>,
    broker_url: Uri
}

impl GetCertsFromBroker {
    async fn query(&self, path: String) -> Result<String,SamplyBeamError> {
        let uri = Uri::builder()
            .scheme(self.broker_url.scheme().unwrap().to_owned())
            .authority(self.broker_url.authority().unwrap().to_owned())
            .path_and_query(path)
            .build()?;
        let mut req = self.client.get(uri).await?;
        let resp = hyper::body::to_bytes(req.body_mut()).await?;
        let resp = String::from_utf8(resp.to_vec())
            .map_err(|e| SamplyBeamError::HttpParseError(e))?;
        if ! req.status().is_success() {
            match req.status() {
                StatusCode::NOT_FOUND => Ok(String::new()),
                x => Err(SamplyBeamError::VaultError(format!("Got code {x}, error message: {}", resp)))
            }
        } else {
            Ok(resp)
        }
    }

    async fn query_vec(&self, path: String) -> Result<Vec<String>,SamplyBeamError> {
        let uri = Uri::builder()
            .scheme(self.broker_url.scheme().unwrap().to_owned())
            .authority(self.broker_url.authority().unwrap().to_owned())
            .path_and_query(path)
            .build()?;
        let mut req = self.client.get(uri).await?;
        let resp = hyper::body::to_bytes(req.body_mut()).await?;
        let json: Vec<String> = serde_json::from_slice(&resp)
            .map_err(|e| SamplyBeamError::VaultError(format!("Unable to parse vault reply: {}", e)))?;
        if ! req.status().is_success() {
            match req.status() {
                StatusCode::NOT_FOUND => Ok(json),
                x => Err(SamplyBeamError::VaultError(format!("Got code {x}")))
            }
        } else {
            Ok(json)
        }
    }
}

#[async_trait]
impl GetCerts for GetCertsFromBroker {
    async fn certificate_list(&self) -> Result<Vec<String>,SamplyBeamError> {
        self.query_vec("/v1/pki/certs".to_string()).await
    }

    async fn certificate_by_serial_as_pem(&self, serial: &str) -> Result<String,SamplyBeamError> {
        debug!("Retrieving certificate with serial {serial} ...");
        self.query(format!("/v1/pki/certs/by_serial/{}", serial)).await
    }

    fn new() -> Result<Self,SamplyBeamError> {
        unimplemented!()
    }
}

pub(crate) fn build_cert_getter(
    config: Config, 
    client: Client<ProxyConnector<HttpsConnector<HttpConnector>>>
) -> Result<GetCertsFromBroker,SamplyBeamError> {
    let client = client;
    let broker_url = config.broker_uri;
    let _ = broker_url.scheme().ok_or(SamplyBeamError::ConfigurationFailed("Broker URL invalid.".into()))?;
    let _ = broker_url.authority().ok_or(SamplyBeamError::ConfigurationFailed("Broker URL invalid.".into()))?;
    // let broker_builder = Uri::builder()
    //     .scheme(scheme)
    //     .authority(authority);
    Ok(GetCertsFromBroker { client, broker_url })
}