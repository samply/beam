use std::{fs, path::PathBuf};

use axum::{body::Bytes, http::{header, request, Method, Request, StatusCode, Uri}, response::Response, Json};
use beam_lib::{AppOrProxyId, ProxyId};
use rsa::{pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey}, pkcs8::DecodePrivateKey, RsaPrivateKey, RsaPublicKey};
use shared::{
    async_trait, crypto::{self, asn_str_to_vault_str, get_all_certs_and_clients_by_cname_as_pemstr, get_best_own_certificate, x509_cert_to_x509_public_key, CryptoPublicPortion, GetCerts, ProxyCertInfo}, errors::{CertificateInvalidReason, SamplyBeamError}, http_client::SamplyHttpClient, jwt_simple::prelude::RS256KeyPair, openssl::x509::X509, reqwest, EncryptedMessage, MsgEmpty
};
use tracing::{debug, info, warn, error};

use crate::{config::{self, Config}, serve_tasks::sign_request};

pub(crate) struct GetCertsFromBroker {
    client: SamplyHttpClient,
    config: Config,
}

impl GetCertsFromBroker {
    pub fn new(client: SamplyHttpClient, config: Config) -> Self {
        Self { client, config }
    }

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

        let req = sign_request(body, parts, &self.config)
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

pub async fn init_public_crypto_for_proxy(
    config: &Config
) -> Result<(ProxyCertInfo, config::ConfigCrypto), SamplyBeamError> {
    let (public_info, new_crypto) = load_public_crypto_for_proxy(config).await?;

    let cert_info = ProxyCertInfo::try_from(&public_info.cert)?;
    Ok((cert_info, new_crypto))
}

pub async fn load_public_crypto_for_proxy(
    config: &Config,
) -> Result<(CryptoPublicPortion, config::ConfigCrypto), SamplyBeamError> {
    let publics: Vec<CryptoPublicPortion> = get_all_certs_and_clients_by_cname_as_pemstr(&config.proxy_id)
        .await
        .into_iter()
        .filter_map(|r| {
            r.map_err(|e| debug!("Unable to parse Certificate: {e}"))
                .ok()
        })
        .collect();
    let public = get_best_own_certificate(publics, &config.crypto.privkey_rsa).ok_or(
        SamplyBeamError::SignEncryptError(
            "Unable to choose valid, newest certificate for this proxy".into(),
        ),
    )?;
    let serial = asn_str_to_vault_str(public.cert.serial_number())?;
    let mut crypto_with_kid = config.crypto.clone();
    crypto_with_kid.privkey_rs256 = crypto_with_kid.privkey_rs256.with_key_id(&serial);
    Ok((public, crypto_with_kid))
}