use aes_gcm::aead::generic_array::GenericArray;
use axum::{Json, http::Request, body::Body};

use static_init::dynamic;
use tokio::{sync::{RwLock, mpsc, oneshot}};
use tracing::{debug, warn, info, error};
use std::{path::Path, error::Error, time::{SystemTime, Duration}, collections::HashMap, sync::Arc, fs::read_to_string};
use rsa::{PublicKey, RsaPrivateKey, RsaPublicKey, PaddingScheme, pkcs1::DecodeRsaPublicKey};
use sha2::{Sha256, Digest};
use openssl::{x509::X509, string::OpensslString, asn1::{Asn1Time, Asn1TimeRef}, error::ErrorStack, rand::rand_bytes};
use vaultrs::{client::{VaultClient, VaultClientSettingsBuilder},pki};

use crate::{errors::SamplyBeamError, config};
use dataobjects::{MsgTaskRequest, EncryptedMsgTaskRequest, beam_id::{ProxyId, BeamId}};

type Serial = String;
const SIGNATURE_LENGTH: u16 = 256;

pub(crate) struct CertificateCache{
    serial_to_x509: HashMap<Serial, X509>,
    cn_to_serial: HashMap<ProxyId, Serial>,
    vault_client: VaultClient,
    pki_realm: String,
    update_trigger: mpsc::Sender<oneshot::Sender<Result<(),SamplyBeamError>>>,
}

impl CertificateCache {
    pub fn new(update_trigger: mpsc::Sender<oneshot::Sender<Result<(),SamplyBeamError>>>) -> Result<CertificateCache,SamplyBeamError> {
        Ok(Self{
        serial_to_x509: HashMap::new(),
        cn_to_serial: HashMap::new(),
        vault_client: VaultClient::new(
            VaultClientSettingsBuilder::default()
                .address(&config::CONFIG_SHARED.pki_address.to_string())
                .token(&config::CONFIG_SHARED.pki_apikey)
                .build()?)?,
        pki_realm: config::CONFIG_SHARED.pki_realm.clone(),
        update_trigger
    })
    }

    /// Searches cache for a certificate with the given ClientId. If not found, updates cache from central vault. If then still not found, return None
    pub async fn get_by_cname(cname: &ProxyId) -> Option<X509> { // TODO: What if multiple certs are found?
        debug!("Getting cert with cname {}", cname);
        { // TODO: Do smart caching: Return reference to existing certificate that exists only once in memory.
            let cache = CERT_CACHE.read().await;
            let cert = match cache.cn_to_serial.get(cname){
                Some(serial) => cache.serial_to_x509.get(serial),
                None => None
            };
            match cert { // why is this not done in the second try?
                Some(certificate) => if x509_date_valid(&certificate).unwrap_or(false) { return cert.cloned(); },
                None => ()
            }
        } // Drop Read Locks
        debug!("Cert with cname {} not found in cache, triggering update.", cname);
        Self::update_certificates().await.unwrap_or(()); // requires write lock. We don't care about the result; cache lookup will just fail.
        let cache = CERT_CACHE.read().await;
        match cache.cn_to_serial.get(cname){
            Some(serial) => cache.serial_to_x509.get(serial).cloned(),
            None => {
                warn!("Unable to find cert {} even after update.", cname);
                None
            }
        }

    }

    /// Searches cache for a certificate with the given Serial. If not found, updates cache from central vault. If then still not found, return None
    pub async fn get_by_serial(serial: &str) -> Option<X509> {
        { // TODO: Do smart caching: Return reference to existing certificate that exists only once in memory.
            let cache = CERT_CACHE.read().await;
            let cert = cache.serial_to_x509.get(serial);
            match cert { // why is this not done in the second try?
                Some(certificate) => if x509_date_valid(&certificate).unwrap_or(false) { return cert.cloned(); },
                None => ()
            }
        }
        Self::update_certificates().await.unwrap_or(()); // requires write lock. We don't care about the result; cache lookup will just fail.
        let cache = CERT_CACHE.read().await;
        return cache.serial_to_x509.get(serial).cloned();
    }

    /// Manually update cache from fetching all certs from the central vault
    async fn update_certificates() -> Result<(),SamplyBeamError> {
        debug!("Triggering certificate update ...");
        let (tx, rx) = oneshot::channel::<Result<(),SamplyBeamError>>();
        CERT_CACHE.read().await.update_trigger.send(tx).await
            .expect("Internal Error: Certificate Store Updater is not listening for requests.");
        match rx.await {
            Ok(result) => {
                debug!("Certificate update successfully completed.");
                result
            },
            Err(e) => Err(SamplyBeamError::InternalSynchronizationError(e.to_string()))
        }
    }

    async fn update_certificates_mut(&mut self) -> Result<(),SamplyBeamError> {
        info!("Updating certificates ...");
        let certificate_list = pki::cert::list(&self.vault_client, &self.pki_realm).await?;
        let new_certificate_serials: Vec<&String> = {
            certificate_list.iter()
                .filter(|serial| !self.serial_to_x509.contains_key(*serial))
                .collect()
        };
        debug!("Received {} certificates ({} of which were new).", certificate_list.len(), new_certificate_serials.len());
        //TODO Check for validity
        for serial in new_certificate_serials {
            let certificate = pki::cert::read(&self.vault_client, &self.pki_realm, serial).await?;
            let opensslcert = X509::from_pem(certificate.certificate.as_bytes())?;
            let commonnames: Vec<ProxyId> = 
                opensslcert.subject_name()
                .entries()
                .map(|x| x.data().as_utf8().unwrap()) // TODO: Remove unwrap, e.g. by supplying empty _or-string
                .collect::<Vec<OpensslString>>()
                .iter()
                .map(|x| {
                    ProxyId::new(&x.to_string())
                        .expect(&format!("Internal error: Vault returned certificate with invalid common name: {}", x))
                })
                .collect();

        if !commonnames.is_empty() && x509_date_valid(&opensslcert)? { // TODO: Check against CA
            self.serial_to_x509.insert(serial.clone(), opensslcert);
            self.cn_to_serial.insert(commonnames.first()
                .expect("Internal error: common names empty; this should not happen")
                .clone(), serial.clone()); //Emptyness is already checked
        }
    }
    Ok(())

    }

    /// Returns all ClientIds and associated certificates currently in cache
    pub async fn get_all_cnames_and_certs() -> Vec<(ProxyId,X509)> {
        let cache = &CERT_CACHE.read().await.serial_to_x509;
        let alias = &CERT_CACHE.read().await.cn_to_serial;
        let mut result = Vec::new();
        if alias.is_empty() {return result;}
        for (cname,serial) in alias.iter() {
            match cache.get(serial) {
                Some(cert) => result.push((cname.clone(), cert.clone())),
                None => ()
            };
        }
        result
    }
}

#[dynamic(lazy)]
static CERT_CACHE: Arc<RwLock<CertificateCache>> = {
    let (tx, mut rx) = mpsc::channel::<oneshot::Sender<Result<(),SamplyBeamError>>>(1);
    let cc = Arc::new(RwLock::new(CertificateCache::new(tx).unwrap()));
    let cc2 = cc.clone();
    tokio::task::spawn(async move {
        while let Some(sender) = rx.recv().await {
            let mut locked_cache = cc2.write().await;
            let result = locked_cache.update_certificates_mut().await;
            if let Err(_) = sender.send(result) {
                warn!("Unable to inform requesting thread that CertificateCache has been updated. Maybe it stopped?");
            }
        }
    });
    cc
};

async fn get_cert_by_serial(serial: &str) -> Option<X509>{
    match CertificateCache::get_by_serial(serial).await {
        Some(x) => Some(x.clone()),
        None => None
    }
}

async fn get_cert_by_cname(cname: &ProxyId) -> Option<X509>{
    match CertificateCache::get_by_cname(cname).await {
        Some(x) => Some(x.clone()),
        None => None
    }
}

#[derive(Debug)]
pub struct ProxyPublicPortion {
    pub beam_id: ProxyId,
    pub cert: String,
    pub pubkey: String,
}

pub async fn get_cert_and_client_by_cname_as_pemstr(cname: &ProxyId) -> Option<ProxyPublicPortion> {
    let cert: Option<X509> = get_cert_by_cname(cname).await;
    extract_x509(cert)
}

// pub async fn get_cert_and_client_by_serial_as_pemstr(serial: &str) -> Option<CertAndClient> {
//     let cert = get_cert_by_serial(serial).await;
//     extract_cert_and_client(cert)
// }

fn extract_x509(cert: Option<X509>) -> Option<ProxyPublicPortion> {
    if cert.is_none() {
        return None;
    }
    let cert = cert.unwrap();

    // Public key
    let pubkey = cert.public_key();
    if pubkey.is_err() {
        error!(?pubkey);
        return None;
    }
    let pubkey = pubkey.unwrap().public_key_to_pem();
    if pubkey.is_err() {
        error!(?pubkey);
        return None;
    }
    let pubkey = std::str::from_utf8(&pubkey.unwrap()).unwrap().to_string();

    let cn = cert.subject_name().entries().next();
    if cn.is_none() {
        return None;
    }
    let verified_sender = cn
        .and_then(|s| Some(s.data()))
        .and_then(|s| match s.as_utf8() {
            Ok(s) => Some(s),
            Err(_) => None
        })
        .and_then(|s| Some(s.to_string()));
    let verified_sender = match verified_sender {
        None => { return None; },
        Some(x) => {
            match ProxyId::new(&x) {
                Ok(x) => x,
                Err(_) => { return None; }
            }
        }
    };
    let cert = cert.to_pem();
    if cert.is_err() {
        return None;
    }
    let cert = cert.unwrap();
    let cert = std::str::from_utf8(cert.as_slice());
    if cert.is_err() {
        return None;
    }
    Some(ProxyPublicPortion {
        beam_id: verified_sender,
        cert: cert.unwrap().into(),
        pubkey,
    })
}

pub(crate) fn hash(data: &[u8]) -> Result<[u8; 32],SamplyBeamError> {
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let digest = hasher.finalize();
    let digest: [u8; 32] = digest[0..32].try_into().unwrap();
    Ok(digest)
}

/// Sign a message with private key. Prepends signature to payload
pub(crate) fn sign(data: &[u8], private_key: &RsaPrivateKey) -> Result<Vec<u8>, SamplyBeamError> {
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let digest = hasher.finalize();
    let mut sig = private_key.sign(PaddingScheme::new_pkcs1v15_sign(Some(rsa::hash::Hash::SHA2_256)), &digest)
        .map_err(|_| SamplyBeamError::SignEncryptError("Unable to sign message.".into()))?;
    assert!(sig.len() as u16 == SIGNATURE_LENGTH); 
    let mut payload: Vec<u8> = data.to_vec();
    sig.append(&mut payload);
    Ok(sig)
}

/// Encrypt the given fields in the json object
pub(crate) async fn encrypt(task: &MsgTaskRequest, fields: &Vec<&str>) -> Result<EncryptedMsgTaskRequest, SamplyBeamError> {
    // TODO: Refactor and complete
    let mut symmetric_key = [0;256];
    let mut nonce = [0;12];
    openssl::rand::rand_bytes(&mut symmetric_key)?;
    openssl::rand::rand_bytes(&mut nonce)?;

    //let receiver_certs: Vec<Option<X509>> = {
        //let mut a = Vec::new();
        // for receiver_cert in task.to {
        //     let cert = CERT_CACHE.get_by_cname(&receiver_cert).await;
        //     a.push(cert);
        // };
        //a
    //};

    // let mut rng = rand::thread_rng();
    // let padding_scheme = PaddingScheme::new_oaep::<sha2::Sha256>();
    // let encrypted_keys = receiver_certs.iter()
    //     .map(|cert| match cert {
    //         Some(c) => Some(c.public_key()?.public_key_to_pem()?),
    //         None => None,
    //     })
    //     .encrypt(&mut rng, PaddingScheme::new_oaep::<sha2::Sha256>(), symmetric_key)
    //     .or_else(|e| Err(SamplyBrokerError::SignEncryptError(&e.to_string())));
    Err(SamplyBeamError::SignEncryptError("Not implemented".into()))
}

/// Encryption method without operation
async fn nop_encrypt(json: serde_json::Value, fields_: &Vec<&str>) -> Result<serde_json::Value, SamplyBeamError> {
    Ok(json)
}

/// Entry point for web framework to encrypt and sing payload
pub(crate) async fn sign_and_encrypt(req: &mut Request<Body>, encrypt_fields: &Vec<&str>) -> Result<(),SamplyBeamError> {

    // Body -> JSON
    let body_json = serde_json::from_str(r#"{"to": ["recipeint1", "recipient2"],}"#)
        .or_else(|e| Err(SamplyBeamError::SignEncryptError("Error serializing request".into())))?;
    let body = req.body_mut();

    // Encrypt Message
    let encrypted_payload = nop_encrypt(body_json, encrypt_fields).await
        .or_else(|e| Err(SamplyBeamError::SignEncryptError("Cannot encrypt message".into())))?;

    //Sign Message
    let signed_message = sign(&encrypted_payload.to_string().as_bytes(), &config::CONFIG_SHARED.privkey_rsa)?;

    // Place new Body in Request
    let new_body = Body::from(signed_message);
    *req.body_mut() = new_body;

    Ok(())
}

//FIXME: Fix slice range compile error
///// Receives a signed payload and returns the verified signer (as ClientID) and the payload
//async fn validate_and_split(raw_bytes: &[u8]) -> Result<(ClientId, &[u8]),SamplyBrokerError>{
    //let signature = raw_bytes[0..SIGNATURE_LENGTH];
    //let payload = raw_bytes[SIGNATURE_LENGTH..];
    //let mut hasher = Sha256::new();
    //hasher.update(&payload);
    //let digest = hasher.finalize();
    //for (client, cert) in CertificateCache::get_all_cnames_and_certs().await {
        //let rsa_key = x509_cert_to_rsa_pub_key(&cert);
        //let result =match rsa_key {
            //Ok(pub_key) => pub_key.verify(PaddingScheme::new_pkcs1v15_sign(Some(rsa::hash::Hash::SHA2_256)), &digest, signature).or_else(|e| Err(SamplyBrokerError::from(e))),
            //Err(e) => Err(e)
        //};
        //if result.is_ok() {return Ok((client, payload));}
    //}
    //Err(SamplyBrokerError::SignEncryptError("Invalid Signature"))
//}

/* Utility Functions */

/// Extracts the pem-encoded public key from a x509 certificate
fn x509_cert_to_x509_public_key(cert: &X509) -> Result<Vec<u8>, SamplyBeamError> {
    match cert.public_key() {
        Ok(key) => key.public_key_to_pem().or_else(|_| Err(SamplyBeamError::SignEncryptError("Invalid public key in x509 certificate.".into()))),
        Err(_) => Err(SamplyBeamError::SignEncryptError("Unable to extract public key from certificate.".into()))
    }
}

/// Converts the x509 pem-encoded public key to the rsa public key
fn x509_public_key_to_rsa_pub_key(cert_key: &Vec<u8>) -> Result<RsaPublicKey, SamplyBeamError> {
    let rsa_key = 
        RsaPublicKey::from_pkcs1_pem(
            std::str::from_utf8(cert_key)
            .or_else(|e| Err(SamplyBeamError::SignEncryptError("Invalid character in certificate public key".into())))?)
        .or_else(|e| Err(SamplyBeamError::SignEncryptError("Can not extract public rsa key from certificate".into())));
    rsa_key
}

/// Convenience function to extract a rsa public key from a x509 certificate. Calls x509_cert_to_x509_public_key and x509_public_key_to_rsa_pub_key internally.
fn x509_cert_to_rsa_pub_key(cert: &X509) -> Result<RsaPublicKey, SamplyBeamError> {
    x509_public_key_to_rsa_pub_key(&x509_cert_to_x509_public_key(cert)?)
}

/// Converts the asn.1 time (e.g., from a certificate exiration date) to rust's SystemTime. From https://github.com/sfackler/rust-openssl/issues/1157#issuecomment-1016737160
fn asn1_time_to_system_time(time: &Asn1TimeRef) -> Result<SystemTime, ErrorStack> {
    let unix_time = Asn1Time::from_unix(0)?.diff(time)?;
    Ok(SystemTime::UNIX_EPOCH + Duration::from_secs(unix_time.days as u64 * 86400 + unix_time.secs as u64))
}

/// Checks if SystemTime::now() is between the not_before and the not_after dates of a x509 certificate
fn x509_date_valid(cert: &X509) -> Result<bool, ErrorStack> {
    let expirydate = asn1_time_to_system_time(cert.not_after())?;
    let startdate = asn1_time_to_system_time(cert.not_before())?;
    let now = SystemTime::now();
    return Ok(expirydate > now && now > startdate);
}
