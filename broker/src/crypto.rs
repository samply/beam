use std::{future::Future, mem::discriminant};

use axum::{
    async_trait,
    http::{method, uri::Scheme},
};
use hyper::{
    body,
    client::{HttpConnector, ResponseFuture},
    header, Body, Client, Method, Request, Response, StatusCode, Uri,
};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use serde::{Deserialize, Serialize};
use shared::{
    config,
    crypto::{GetCerts, CertificateCache, CertificateCacheUpdate, parse_crl},
    errors::SamplyBeamError,
    http_client::{self, SamplyHttpClient}, openssl::x509::X509Crl,
};
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, error, warn, info};

use crate::health::{self, VaultStatus};

pub struct GetCertsFromPki {
    pki_realm: String,
    hyper_client: SamplyHttpClient,
    health_report_sender: tokio::sync::watch::Sender<health::VaultStatus>,
}

#[derive(Debug, Deserialize, Clone, Hash)]
struct KeyHolder {
    keys: Vec<String>,
}
#[derive(Debug, Deserialize, Clone, Hash)]
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

impl GetCertsFromPki {
    pub(crate) fn new(
        health_report_sender: tokio::sync::watch::Sender<health::VaultStatus>,
    ) -> Result<Self, SamplyBeamError> {
        let mut certs: Vec<String> = Vec::new();
        if let Some(dir) = &config::CONFIG_CENTRAL.tls_ca_certificates_dir {
            for file in std::fs::read_dir(dir).map_err(|e| {
                SamplyBeamError::ConfigurationFailed(format!(
                    "Unable to read CA certificates: {}",
                    e
                ))
            })? {
                if let Ok(file) = file {
                    certs.push(file.path().to_str().unwrap().into());
                }
            }
            debug!("Loaded local certificates: {}", certs.join(" "));
        }
        let hyper_client = http_client::build(
            &config::CONFIG_SHARED.tls_ca_certificates,
            Some(Duration::from_secs(30)),
            Some(Duration::from_secs(20)),
        )
        .map_err(SamplyBeamError::HttpProxyProblem)?;
        let pki_realm = config::CONFIG_CENTRAL.pki_realm.clone();

        Ok(Self {
            pki_realm,
            hyper_client,
            health_report_sender,
        })
    }

    async fn report_vault_health(&self, status: VaultStatus) {
        self.health_report_sender.send_if_modified(|val| {
            if discriminant(val) != discriminant(&status) {
                *val = status;
                true
            } else {
                false
            }
        });
    }

    pub(crate) async fn check_vault_health(&self) -> Result<(), SamplyBeamError> {
        let state = self.check_vault_health_helper().await;
        let monitoring_status = match state {
            Ok(_) => VaultStatus::Ok,
            Err(ref e) => match e {
                SamplyBeamError::VaultSealed | SamplyBeamError::VaultNotInitialized => {
                    VaultStatus::LockedOrSealed
                }
                SamplyBeamError::VaultUnreachable(_) => VaultStatus::Unreachable,
                _ => VaultStatus::OtherError,
            },
        };
        self.report_vault_health(monitoring_status).await;
        state
    }

    async fn check_vault_health_helper(&self) -> Result<(), SamplyBeamError> {
        let url = pki_url_builder("sys/health");
        debug!("Checking Vault's health at URL {url}");
        let health = self.hyper_client.get(url).await;
        let Ok(resp) = health else {
            return Err(SamplyBeamError::VaultUnreachable(health.unwrap_err()));
        };
        match resp.status() {
            code if code.is_success() => Ok(()),
            code if code.is_redirection() => {
                let location = resp.headers().get(header::LOCATION);
                let location = match location {
                    Some(x) => x.to_str().unwrap_or("(garbled Location header)"),
                    None => "(no Location header present)",
                };
                Err(SamplyBeamError::VaultRedirectError(code, location.into()))
            }
            StatusCode::NOT_IMPLEMENTED => Err(SamplyBeamError::VaultNotInitialized),
            StatusCode::SERVICE_UNAVAILABLE => Err(SamplyBeamError::VaultSealed),
            code => Err(SamplyBeamError::VaultOtherError(format!(
                "Vault healthcheck returned statuscode {}",
                code
            ))),
        }
    }

    async fn resilient_vault_request(
        &self,
        method: &Method,
        api_path: &str,
        max_tries: Option<u32>,
    ) -> Result<Response<Body>, SamplyBeamError> {
        debug!("Samply.PKI: Vault request to {api_path}");
        let uri = pki_url_builder(api_path);
        let max_tries = max_tries.unwrap_or(u32::MAX);
        for tries in 0..max_tries {
            if tries > 0 {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            let req = Request::builder()
                .method(method)
                .header("X-Vault-Token", &config::CONFIG_CENTRAL.pki_token)
                .uri(&uri)
                .header("User-Agent", env!("SAMPLY_USER_AGENT"))
                .body(body::Body::empty())
                .unwrap(); //TODO Unwrap
            let resp = self.hyper_client.request(req).await;
            let Ok(resp) = resp else {
                warn!("Samply.PKI: Unable to communicate to vault: {}; retrying (failed attempt #{})", resp.unwrap_err(), tries+2);
                self.report_vault_health(VaultStatus::Unreachable).await;
                continue;
            };
            match resp.status() {
                code if code.is_success() => {
                    self.report_vault_health(VaultStatus::Ok).await;
                    return Ok(resp);
                }
                code if code.is_client_error() || code.is_redirection() => {
                    error!(
                        "Samply.PKI: Vault reported client-side Error (code {}), not retrying.",
                        code
                    );
                    self.report_vault_health(VaultStatus::OtherError).await;
                    return Err(SamplyBeamError::VaultOtherError(format!(
                        "Samply.PKI: Vault reported client-side error (code {})",
                        code
                    )));
                }
                code => {
                    match self.check_vault_health().await {
                        Err(SamplyBeamError::VaultSealed) => {
                            warn!(
                                "Samply.PKI: Vault is still sealed; retrying (failed attempt {})",
                                tries
                            );
                            continue;
                        }
                        Err(SamplyBeamError::VaultRedirectError(code, location)) => {
                            let err = SamplyBeamError::VaultRedirectError(code, location);
                            error!("Samply.PKI asked to redirect; aborting: {err}");
                            return Err(err);
                        }
                        Err(e) => {
                            warn!("Samply.PKI: Got error from Vault: {}; status code {}; retrying (failed attempt #{})", e, code, tries);
                            continue;
                        }
                        Ok(()) => {
                            debug!("Got code {} communicating with Samply.PKI fetching URL {api_path}.", code);
                            continue;
                        }
                    }
                }
            }
        }
        let err = format!(
            "Samply.PKI: Unable to communicate after {} attempts. Giving up.",
            max_tries
        );
        error!(err);
        Err(SamplyBeamError::VaultOtherError(err))
    }
}

#[async_trait]
impl GetCerts for GetCertsFromPki {
    async fn certificate_list_via_network(&self) -> Result<Vec<String>, SamplyBeamError> {
        debug!("Getting Cert List via network");
        let resp = self
            .resilient_vault_request(
                &Method::from_bytes("LIST".as_bytes()).unwrap(),
                &format!("{}/certs", &config::CONFIG_CENTRAL.pki_realm),
                Some(100),
            )
            .await?;
        let body_bytes = body::to_bytes(resp.into_body()).await.map_err(|e| {
            SamplyBeamError::VaultOtherError(format!(
                "Cannot retrieve vault certificate list: {}",
                e
            ))
        })?;
        let body: PkiListResponse = serde_json::from_slice(&body_bytes).map_err(|e| {
            SamplyBeamError::VaultOtherError(format!(
                "Cannot deserialize vault certificate list: {}",
                e
            ))
        })?;
        debug!("Got cert list with {} elements", body.data.keys.len());
        return Ok(body.data.keys);
    }

    async fn certificate_by_serial_as_pem(&self, serial: &str) -> Result<String, SamplyBeamError> {
        debug!("Getting Cert with serial {}", serial);
        let resp = self
            .resilient_vault_request(
                &Method::GET,
                &format!("{}/cert/{}/raw/pem", &self.pki_realm, serial),
                Some(100),
            )
            .await?;
        let body_bytes = body::to_bytes(resp.into_body()).await.map_err(|e| {
            SamplyBeamError::VaultOtherError(format!(
                "Cannot retrieve certificate {}: {}",
                serial, e
            ))
        })?;
        let body = String::from_utf8(body_bytes.to_vec()).map_err(|e| {
            SamplyBeamError::VaultOtherError(format!("Cannot parse certificate {}: {}", serial, e))
        })?;
        return Ok(body);
    }

    async fn im_certificate_as_pem(&self) -> Result<String, SamplyBeamError> {
        debug!("Getting IM CA Cert");
        let resp = self
            .resilient_vault_request(
                &Method::GET,
                &format!("{}/ca/pem", self.pki_realm),
                Some(100),
            )
            .await?;
        let body_bytes = body::to_bytes(resp.into_body()).await.map_err(|e| {
            SamplyBeamError::VaultOtherError(format!("Cannot retrieve im-ca certificate: {}", e))
        })?;
        let body = String::from_utf8(body_bytes.to_vec()).map_err(|e| {
            SamplyBeamError::VaultOtherError(format!("Cannot parse im-ca certificate: {}", e))
        })?;
        return Ok(body);
    }

    async fn on_timer(&self, cache: &mut CertificateCache) -> CertificateCacheUpdate {
        let result = cache.update_certificates_mut().await;
        match result {
            Err(e) => {
                warn!("Unable to update CertificateCache. Maybe it stopped? Reason: {e}.");
                CertificateCacheUpdate::UnChanged
            }
            Ok(update) => update
        }
    }

    async fn get_crl(&self) -> Result<Option<X509Crl>, SamplyBeamError> {
        debug!("Getting crl");
        let resp = self.resilient_vault_request(
            &Method::GET,
            &format!("{}/crl", self.pki_realm),
            Some(100),
        )
        .await?;
        let body_bytes = body::to_bytes(resp.into_body()).await.map_err(|e| {
            SamplyBeamError::VaultOtherError(format!("Cannot retrieve crl: {}", e))
        })?;
        parse_crl(&body_bytes).map(Some)
    }
}

pub(crate) fn build_cert_getter(
    sender: tokio::sync::watch::Sender<VaultStatus>,
) -> Result<GetCertsFromPki, SamplyBeamError> {
    GetCertsFromPki::new(sender)
}

pub(crate) fn pki_url_builder(location: &str) -> Uri {
    Uri::builder()
        .scheme(
            config::CONFIG_CENTRAL
                .pki_address
                .scheme()
                .unwrap()
                .as_str(),
        )
        .authority(
            config::CONFIG_CENTRAL
                .pki_address
                .authority()
                .unwrap()
                .to_owned(),
        )
        .path_and_query(format!("/v1/{}", location))
        .build()
        .unwrap() // TODO Unwrap
}
