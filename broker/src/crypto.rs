use std::future::Future;

use axum::async_trait;
use hyper::{Uri, Request, client::HttpConnector, Client, header, body, StatusCode};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use serde::{Serialize, Deserialize};
use shared::{crypto::GetCerts, errors::SamplyBeamError, config};
use tracing::debug;

pub struct GetCertsFromPki {
    pki_realm: String,
    hyper_client: Client<ProxyConnector<HttpsConnector<HttpConnector>>>
}

#[derive(Debug,Deserialize,Clone,Hash)]
struct KeyHolder {keys: Vec<String>}
#[derive(Debug,Deserialize,Clone,Hash)]
struct PkiListResponse {
    request_id: String,
    lease_id: String,
    renewable: bool,
    lease_duration: u32,
    data: KeyHolder,
    wrap_info: Option<String>,
    warnings: Option<String>,
    auth: Option<String>,
}

#[async_trait]
impl GetCerts for GetCertsFromPki {
    async fn certificate_list(&self) -> Result<Vec<String>,SamplyBeamError> {
        debug!("Getting Cert List");
        let uri = pki_url_builder(&format!("{}/certs",&config::CONFIG_CENTRAL.pki_realm));
        let req = Request::builder()
            .method("LIST")
            .header("X-Vault-Token",&config::CONFIG_CENTRAL.pki_token)
            .header("User-Agent", &format!("beam.broker/{}",env!("CARGO_PKG_VERSION")))
            .uri(uri)
            .body(body::Body::empty()).expect("Can not create Cert List Request"); //TODO Unwrap
        let resp = self.hyper_client.request(req).await
            .map_err(|e| SamplyBeamError::VaultError(format!("Cannot connect to vault: {}",e)))?;
        if resp.status() == StatusCode::OK {
            let body_bytes = body::to_bytes(resp.into_body()).await
                .map_err(|e| SamplyBeamError::VaultError(format!("Cannot retreive vault certificate list: {}",e)))?;
            let body: PkiListResponse = serde_json::from_slice(&body_bytes)
                .map_err(|e| SamplyBeamError::VaultError(format!("Cannot deserialize vault certificate list: {}",e)))?;
            debug!("Got cert list with {} elements",body.data.keys.len());
            return Ok(body.data.keys);
        }
        debug!("No cert list! Status {}", resp.status());
        Err(SamplyBeamError::VaultError(format!("Cannot retreive certificate list: {}",resp.status())))
    }

    async fn certificate_by_serial_as_pem(&self, serial: &str) -> Result<String,SamplyBeamError> {
        debug!("Getting Cert with serial {}",serial);
        let uri = pki_url_builder(&format!("{}/cert/{}/raw/pem",&self.pki_realm, serial));
        let req = Request::builder()
            .method("GET")
            .header("X-Vault-Token",&config::CONFIG_CENTRAL.pki_token)
            .uri(uri)
            .body(body::Body::empty()).unwrap(); //TODO Unwrap
        let resp = self.hyper_client.request(req).await
            .map_err(|e| SamplyBeamError::VaultError(format!("Cannot connect to vault: {}",e)))?;
        if resp.status() == StatusCode::OK {
            let body_bytes = body::to_bytes(resp.into_body()).await
            .map_err(|e| SamplyBeamError::VaultError(format!("Cannot retreive certificate {}: {}",serial,e)))?;
            let body = String::from_utf8(body_bytes.to_vec())
                .map_err(|e| SamplyBeamError::VaultError(format!("Cannot parse certificate {}: {}",serial,e)))?;
            return Ok(body);
        }
        Err(SamplyBeamError::VaultError(format!("Cannot retreive certificate {}: {}",serial, resp.status())))
    }

    async fn im_certificate_as_pem(&self) -> Result<String,SamplyBeamError> {
        debug!("Getting IM CA Cert");
        let uri = pki_url_builder(&format!("{}/ca/pem",&self.pki_realm));
        let req = Request::builder()
            .method("GET")
            .header("X-Vault-Token",&config::CONFIG_CENTRAL.pki_token)
            .uri(uri)
            .body(body::Body::empty()).unwrap(); //TODO Unwrap
        let resp = self.hyper_client.request(req).await
            .map_err(|e| SamplyBeamError::VaultError(format!("Cannot connect to vault: {}",e)))?;
        if resp.status() == StatusCode::OK {
            let body_bytes = body::to_bytes(resp.into_body()).await
            .map_err(|e| SamplyBeamError::VaultError(format!("Cannot retreive im-ca certificate: {}",e)))?;
            let body = String::from_utf8(body_bytes.to_vec())
                .map_err(|e| SamplyBeamError::VaultError(format!("Cannot parse im-ca certificate: {}",e)))?;
            return Ok(body);
        }
        Err(SamplyBeamError::VaultError(format!("Cannot retreive im-ca certificate: {}",resp.status())))
    }

    fn new() -> Result<Self,SamplyBeamError> {
        let mut certs: Vec<String> = Vec::new();
        if let Some(dir) = &config::CONFIG_CENTRAL.tls_ca_certificates_dir {
            for file in std::fs::read_dir(dir).map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Unable to read CA certificates: {}", e)))? {
                if let Ok(file) = file {
                    certs.push(file.path().to_str().unwrap().into());
                }
            }
            debug!("Loaded local certificates: {}", certs.join(" "));
        }
        let hyper_client = shared::http_proxy::build_hyper_client(&config::CONFIG_SHARED.tls_ca_certificates)
            .map_err(SamplyBeamError::HttpProxyProblem)?;
        let pki_realm = config::CONFIG_CENTRAL.pki_realm.clone();

        Ok(Self { pki_realm , hyper_client})
    }
}

pub(crate) fn build_cert_getter() -> Result<GetCertsFromPki,SamplyBeamError> {
    GetCertsFromPki::new()
}

pub(crate) fn pki_url_builder(location: &str) -> Uri {
    Uri::builder().scheme(config::CONFIG_CENTRAL.pki_address.scheme().unwrap().as_str()).authority(config::CONFIG_CENTRAL.pki_address.authority().unwrap().to_owned()).path_and_query(format!("/v1/{}",location)).build().unwrap() // TODO Unwrap
}
