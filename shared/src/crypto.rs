use axum::{async_trait, body::Body, http::Request, Json};

use itertools::Itertools;
use once_cell::sync::OnceCell;
use openssl::{
    asn1::{Asn1Time, Asn1TimeRef},
    error::ErrorStack,
    rand::rand_bytes,
    string::OpensslString,
    x509::X509,
};
use rsa::{
    pkcs1::DecodeRsaPublicKey, pkcs8::DecodePublicKey, PaddingScheme, PublicKey, PublicKeyParts,
    RsaPrivateKey, RsaPublicKey,
};
use sha2::{Digest, Sha256};
use static_init::dynamic;
use std::{
    borrow::BorrowMut,
    collections::HashMap,
    error::Error,
    fs::read_to_string,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::{sync::{mpsc, oneshot, RwLock}, time::Instant};
use tracing::{debug, error, info, warn};

use crate::{
    beam_id::{AppOrProxyId, BeamId, ProxyId},
    config,
    config_shared::ConfigCrypto,
    crypto,
    errors::{CertificateInvalidReason, SamplyBeamError},
    EncryptedMsgTaskRequest, MsgTaskRequest,
};

type Serial = String;

pub(crate) struct ProxyCertInfo {
    pub(crate) proxy_name: String,
    pub(crate) valid_since: String,
    pub(crate) valid_until: String,
    pub(crate) common_name: String,
    pub(crate) serial: String,
}

impl TryFrom<&X509> for ProxyCertInfo {
    type Error = SamplyBeamError;

    fn try_from(cert: &X509) -> Result<Self, Self::Error> {
        // let remaining = Asn1Time::days_from_now(0)?.diff(cert.not_after())?;
        let common_name = cert
            .subject_name()
            .entries()
            .find(|c| c.object().nid() == openssl::nid::Nid::COMMONNAME)
            .ok_or(CertificateInvalidReason::NoCommonName)?
            .data()
            .as_utf8()?
            .to_string();

        const SERIALERR: SamplyBeamError =
            SamplyBeamError::CertificateError(CertificateInvalidReason::WrongSerial);

        let certinfo = ProxyCertInfo {
            proxy_name: common_name
                .split('.')
                .next()
                .ok_or(CertificateInvalidReason::InvalidCommonName)?
                .into(),
            common_name,
            valid_since: cert.not_before().to_string(),
            valid_until: cert.not_after().to_string(),
            serial: cert
                .serial_number()
                .to_bn()
                .map_err(|_e| SERIALERR)?
                .to_hex_str()
                .map_err(|_e| SERIALERR)?
                .to_string(),
        };
        Ok(certinfo)
    }
}

#[derive(Clone)]
pub(crate) enum CertificateCacheEntry {
    Valid(X509),
    Invalid(CertificateInvalidReason),
}

pub(crate) struct CertificateCache {
    serial_to_x509: HashMap<Serial, CertificateCacheEntry>,
    cn_to_serial: HashMap<ProxyId, Vec<Serial>>,
    update_trigger: mpsc::Sender<oneshot::Sender<Result<usize, SamplyBeamError>>>,
    root_cert: Option<X509>, // Might not be available at initialization time
    im_cert: Option<X509>,   // Might not be available at initialization time
}

#[async_trait]
pub trait GetCerts: Sync + Send {
    async fn certificate_list(&self) -> Result<Vec<String>, SamplyBeamError>;
    async fn certificate_by_serial_as_pem(&self, serial: &str) -> Result<String, SamplyBeamError>;
    async fn im_certificate_as_pem(&self) -> Result<String, SamplyBeamError>;
    async fn on_cert_expired(&self, _cert: X509) {}
}

impl CertificateCache {
    pub fn new(
        update_trigger: mpsc::Sender<oneshot::Sender<Result<usize, SamplyBeamError>>>,
    ) -> Result<CertificateCache, SamplyBeamError> {
        Ok(Self {
            serial_to_x509: HashMap::new(),
            cn_to_serial: HashMap::new(),
            update_trigger,
            root_cert: None,
            im_cert: None,
        })
    }

    /// Searches cache for a certificate with the given ClientId. If not found, updates cache from central vault. If then still not found, return None
    pub async fn get_all_certs_by_cname(cname: &ProxyId) -> Vec<CertificateCacheEntry> {
        // TODO: What if multiple certs are found?
        let mut result = Vec::new();
        Self::update_certificates().await.unwrap_or_else(|e| {
            // requires write lock.
            warn!("Updating certificates failed: {}", e);
            0
        });
        debug!("Getting cert(s) with cname {}", cname);
        let mut valid = 0;
        let mut invalid = 0;
        {
            // TODO: Do smart caching: Return reference to existing certificate that exists only once in memory.
            let cache = CERT_CACHE.read().await;
            if let Some(serials) = cache.cn_to_serial.get(cname) {
                debug!(
                    "Considering {} certificates with matching CN: {:?}",
                    serials.len(),
                    serials
                );
                for serial in serials {
                    debug!("Fetching certificate with serial {}", serial);
                    let x509 = cache.serial_to_x509.get(serial);
                    if let Some(x509) = x509 {
                        match x509 {
                            CertificateCacheEntry::Invalid(reason) => {
                                result.push(x509.clone());
                                invalid += 1;
                            }
                            CertificateCacheEntry::Valid(x509) => {
                                if !x509_date_valid(x509).unwrap_or(true) {
                                    let Ok(info) = crypto::ProxyCertInfo::try_from(x509) else {
                                        warn!("Found invalid x509 certificate -- even unable to parse it.");
                                        continue;
                                    };
                                    warn!("Found x509 certificate with invalid date: CN={}, serial={}", info.common_name, info.serial);
                                } else {
                                    debug!(
                                        "Certificate with serial {} successfully retrieved.",
                                        serial
                                    );
                                    result.push(CertificateCacheEntry::Valid(x509.clone()));
                                    valid += 1;
                                }
                            }
                        }
                    }
                }
            
            };
        } // Drop Read Locks
        if result.is_empty() {
            warn!(
                "Did not find certificate for cname {}, even after update.",
                cname
            );
        } else {
            debug!(
                "Found {valid} valid and {invalid} invalid certificate(s) for cname {}.",
                cname
            );
        }
        result
    }

    /// Searches cache for a certificate with the given Serial. If not found, updates cache from central vault. If then still not found, return None
    pub async fn get_by_serial(serial: &str) -> Option<CertificateCacheEntry> {
        {
            // TODO: Do smart caching: Return reference to existing certificate that exists only once in memory.
            let cache = CERT_CACHE.read().await;
            let cert = cache.serial_to_x509.get(serial);
            match cert {
                // why is this not done in the second try?
                Some(x) => {
                    return Some(x.clone());
                }
                None => (),
            }
        }
        Self::update_certificates().await.unwrap_or_else(|e| {
            // requires write lock.
            warn!("Updating certificates failed: {}", e);
            0
        });
        let cache = CERT_CACHE.read().await;
        let cert = cache.serial_to_x509.get(serial);

        cert.cloned()
    }

    /// Manually update cache from fetching all certs from the central vault
    async fn update_certificates() -> Result<usize, SamplyBeamError> {
        debug!("Triggering certificate update ...");
        let (tx, rx) = oneshot::channel::<Result<usize, SamplyBeamError>>();
        CERT_CACHE
            .read()
            .await
            .update_trigger
            .send(tx)
            .await
            .expect("Internal Error: Certificate Store Updater is not listening for requests.");
        match rx.await {
            Ok(Ok(result)) => {
                debug!("Certificate update successfully completed: Got {result} new certificates.");
                Ok(result)
            }
            Ok(Err(e)) => {
                error!("Unable to sync certificates: {e}");
                Err(e)
            }
            Err(e) => Err(SamplyBeamError::InternalSynchronizationError(e.to_string())),
        }
    }

    async fn update_certificates_mut(&mut self) -> Result<usize, SamplyBeamError> {
        info!("Updating certificates ...");
        let certificate_list = CERT_GETTER.get().unwrap().certificate_list().await?;
        let new_certificate_serials: Vec<&String> = {
            certificate_list
                .iter()
                .filter(|serial| !self.serial_to_x509.contains_key(*serial))
                .collect()
        };
        debug!(
            "Received {} certificates ({} of which were new).",
            certificate_list.len(),
            new_certificate_serials.len()
        );

        let mut new_count = 0;
        //TODO Check for validity
        for serial in new_certificate_serials {
            debug!("Checking certificate with serial {serial}");

            let certificate = CERT_GETTER
                .get()
                .unwrap()
                .certificate_by_serial_as_pem(serial)
                .await;
            if let Err(e) = certificate {
                match e {
                    SamplyBeamError::CertificateError(err) => {
                        debug!("Will skip invalid certificate {serial} from now on.");
                        self.serial_to_x509
                            .insert(serial.clone(), CertificateCacheEntry::Invalid(err));
                    }
                    other_error => {
                        warn!(
                            "Could not retrieve certificate for serial {serial}: {}",
                            other_error
                        );
                    }
                };
                continue;
            }
            let certificate = certificate.unwrap();
            let opensslcert = match X509::from_pem(certificate.as_bytes()) {
                Ok(x) => x,
                Err(err) => {
                    error!("Skipping unparseable certificate {serial}: {err}");
                    continue;
                }
            };
            let commonnames: Vec<ProxyId> = opensslcert
                .subject_name()
                .entries()
                .map(|x| x.data().as_utf8().unwrap()) // TODO: Remove unwrap, e.g. by supplying empty _or-string
                .collect::<Vec<OpensslString>>()
                .iter()
                .map(|x| {
                    ProxyId::new(&x.to_string()).expect(&format!(
                        "Internal error: Vault returned certificate with invalid common name: {}",
                        x
                    ))
                })
                .collect();

            let err = {
                if commonnames.is_empty() {
                    Some(CertificateInvalidReason::NoCommonName)
                } else if let Err(e) = verify_cert(
                    &opensslcert,
                    &self
                        .im_cert
                        .as_ref()
                        .expect("No intermediate CA cert found"),
                ) {
                    Some(e)
                } else {
                    None
                }
            };
            if let Some(err) = err {
                warn!("Certificate with serial {} invalid: {}.", serial, err);
                self.serial_to_x509
                    .insert(serial.clone(), CertificateCacheEntry::Invalid(err));
            } else {
                let cn = commonnames
                    .first()
                    .expect("Internal error: common names empty; this should not happen");
                self.serial_to_x509
                    .insert(serial.clone(), CertificateCacheEntry::Valid(opensslcert));
                match self.cn_to_serial.get_mut(cn) {
                    Some(serials) => serials.push(serial.clone()),
                    None => {
                        let new = vec![serial.clone()];
                        self.cn_to_serial.insert(cn.clone(), new);
                    }
                };
                debug!("Added certificate {} for cname {}", serial, cn);
                new_count += 1;
            }
        }
        Ok(new_count)
    }

    /*
    /// Returns all ClientIds and associated certificates currently in cache
    pub async fn get_all_cnames_and_certs() -> Vec<(ProxyId,X509)> {
        let cache = &CERT_CACHE.read().await.serial_to_x509;
        let alias = &CERT_CACHE.read().await.cn_to_serial;
        let mut result = Vec::new();
        if alias.is_empty() {
            return result;
        }
        for (cname,serials) in alias.iter() {
            for serial in serials {
                if let Some(cert) = cache.get(serial) {
                    result.push((cname.clone(), cert.clone()));
                } else {
                    warn!("Unable to find certificate for serial {}.", serial);
                }
            }
        }
        result
    }*/

    /// Sets the root certificate, which is usually not available at static time. Must be called before certificate validation
    pub fn set_root_cert(&mut self, root_certificate: &X509) {
        self.root_cert = Some(root_certificate.clone());
    }

    pub async fn set_im_cert(&mut self) -> Result<(), SamplyBeamError> {
        self.im_cert = Some(X509::from_pem(&get_im_cert().await.unwrap().as_bytes())?);
        let _ = verify_cert(&self.im_cert.as_ref().expect("No IM certificate provided"), &self.root_cert.as_ref().expect("No root certificate set!"))
            .expect(&format!("The intermediate certificate is invalid. Please send this info to the central beam admin for debugging:\n---BEGIN DEBUG---\n{}\nroot\n{}\n---END DEBUG---", 
                             String::from_utf8(self.im_cert.as_ref().unwrap().to_text().unwrap_or("Cannot convert IM certificate to text".into())).unwrap_or("Invalid characters in IM certificate".to_string()),
                             String::from_utf8(self.root_cert.as_ref().unwrap().to_text().unwrap_or("Cannot convert root certificate to text".into())).unwrap_or("Invalid characters in root certificate".to_string())));
        Ok(())
    }
}

/// Wrapper for initializing the CA chain. Must be called *after* config initialization
pub async fn init_ca_chain() -> Result<(), SamplyBeamError> {
    let mut cache = CERT_CACHE.write().await;
    cache.set_root_cert(&config::CONFIG_SHARED.root_cert);
    cache.set_im_cert().await?;
    Ok(())
}

static CERT_GETTER: OnceCell<Box<dyn GetCerts>> = OnceCell::new();

pub fn init_cert_getter<G: GetCerts + 'static>(getter: G) {
    let res = CERT_GETTER.set(Box::new(getter));
    if res.is_err() {
        panic!("Internal error: Tried to initialize cert_getter twice");
    }
}

pub async fn get_serial_list() -> Result<Vec<String>, SamplyBeamError> {
    CERT_GETTER.get().unwrap().certificate_list().await
}

pub async fn get_im_cert() -> Result<String, SamplyBeamError> {
    CERT_GETTER.get().unwrap().im_certificate_as_pem().await
}

#[dynamic(lazy)]
pub(crate) static CERT_CACHE: Arc<RwLock<CertificateCache>> = {
    let (tx, mut rx) = mpsc::channel::<oneshot::Sender<Result<usize, SamplyBeamError>>>(1);
    let cc = Arc::new(RwLock::new(CertificateCache::new(tx).unwrap()));
    let cc2 = cc.clone();
    let cc3 = cc.clone();
    tokio::task::spawn(async move {
        while let Some(sender) = rx.recv().await {
            let mut locked_cache = cc2.write().await;
            let result = locked_cache.update_certificates_mut().await;
            match &result {
                Err(e) => {
                    warn!("Unable to inform requesting thread that CertificateCache has been updated. Maybe it stopped? Reason: {e}");
                }
                Ok(count) => {
                    if *count > 0 {
                        info!("Added {count} new certificates.");
                    } else {
                        info!("No new certificates have been found.");
                    }
                }
            };
            if let Err(_err) = sender.send(result) {
                warn!("Unable to inform requesting thread that CertificateCache has been updated. Maybe it stopped?");
            }
        }
    });
    tokio::spawn(async move {
        loop {
            // Get oldest cert
            let old_cert = cc3.read().await
                .serial_to_x509
                .values()
                .filter_map(|entry| if let CertificateCacheEntry::Valid(cert) = entry {
                    Some(cert)
                } else {
                    None
                })
                .min_by(|cert_a, cert_b| cert_a.not_after().compare(cert_b.not_after()).unwrap_or_else(|err| {
                    warn!("Got error sorting certs: {err}");
                    std::cmp::Ordering::Greater
                }))
                .cloned();
            // if we dont have certs yet wait
            let Some(old_cert) = old_cert else {
                tokio::time::sleep(Duration::from_secs(20)).await;
                continue;
            };
            // Sleep until expired
            let expire_date = asn1_time_to_system_time(old_cert.not_after()).unwrap_or(SystemTime::now());
            let duration = (expire_date.duration_since(SystemTime::now())).unwrap_or(Duration::from_secs(0));
            let secs = duration.as_secs();
            info!("Oldest cert will expire in: {}d {}h {}m {}s", secs / (24 * 60 * 60), (secs % (24 * 60 * 60)) / (60 * 60), (secs % (60 * 60)) / 60, secs % 60);
            tokio::time::sleep(duration).await;
            info!("Invalidating old cert {:?} now", old_cert);
            // Invalidate cert in cache
            {
                let mut cache_lock = cc3.write().await;
                let Some(entry) = cache_lock
                    .serial_to_x509
                    .values_mut()
                    .find(|other| if let CertificateCacheEntry::Valid(cert) = other {
                        cert == &old_cert
                    } else {
                        false
                    }
                ) else {
                    warn!("Certificate that has expired was no longer in cache: {old_cert:?}");
                    return;
                };
                *entry = CertificateCacheEntry::Invalid(CertificateInvalidReason::InvalidDate);
            }
            CERT_GETTER.get().unwrap().on_cert_expired(old_cert).await;
        }
    });
    cc
};

async fn get_cert_by_serial(serial: &str) -> Option<CertificateCacheEntry> {
    CertificateCache::get_by_serial(serial).await
}

async fn get_all_certs_by_cname(cname: &ProxyId) -> Vec<CertificateCacheEntry> {
    CertificateCache::get_all_certs_by_cname(cname).await
}

#[derive(Debug, Clone)]
pub struct CryptoPublicPortion {
    pub beam_id: ProxyId,
    pub cert: X509,
    pub pubkey: String,
}

pub async fn get_all_certs_and_clients_by_cname_as_pemstr(
    cname: &ProxyId,
) -> Vec<Result<CryptoPublicPortion, CertificateInvalidReason>> {
    get_all_certs_by_cname(cname)
        .await
        .iter()
        .map(|c| match c {
            CertificateCacheEntry::Valid(c) => extract_x509(c),
            CertificateCacheEntry::Invalid(reason) => Err(reason.clone()),
        })
        .collect()
}

pub async fn get_cert_and_client_by_serial_as_pemstr(
    serial: &str,
) -> Option<Result<CryptoPublicPortion, CertificateInvalidReason>> {
    match get_cert_by_serial(serial).await {
        None => None,
        Some(CertificateCacheEntry::Valid(valid_cert)) => Some(extract_x509(&valid_cert)),
        Some(CertificateCacheEntry::Invalid(reason)) => Some(Err(reason)),
    }
}

pub async fn get_newest_certs_for_cnames_as_pemstr(
    cnames: impl IntoIterator<Item = &ProxyId>,
) -> Option<Vec<CryptoPublicPortion>> {
    let mut result: Vec<CryptoPublicPortion> = Vec::new(); // No fancy map/iter, bc of async
    for id in cnames {
        let certs = get_all_certs_and_clients_by_cname_as_pemstr(id)
            .await
            .into_iter()
            .flatten()
            .collect();
        if let Some(best_candidate) = get_best_other_certificate(&certs) {
            result.push(best_candidate);
        }
    }
    (!result.is_empty()).then_some(result)
}

fn extract_x509(cert: &X509) -> Result<CryptoPublicPortion, CertificateInvalidReason> {
    // Public key
    let pubkey = cert
        .public_key()
        .map_err(|_e| CertificateInvalidReason::InvalidPublicKey)?
        .public_key_to_pem()
        .map_err(|_e| CertificateInvalidReason::InvalidPublicKey)
        .and_then(|v| {
            String::from_utf8(v).map_err(|_e| CertificateInvalidReason::InvalidPublicKey)
        })?;

    let cn = cert.subject_name().entries().next();
    if cn.is_none() {
        return Err(CertificateInvalidReason::NoCommonName);
    }
    let verified_sender = cn
        .and_then(|s| Some(s.data()))
        .and_then(|s| match s.as_utf8() {
            Ok(s) => Some(s),
            Err(_) => None,
        })
        .and_then(|s| Some(s.to_string()));
    let verified_sender = match verified_sender {
        None => return Err(CertificateInvalidReason::InvalidCommonName),
        Some(x) => match ProxyId::new(&x) {
            Ok(x) => x,
            Err(_err) => {
                return Err(CertificateInvalidReason::InvalidCommonName);
            }
        },
    };
    Ok(CryptoPublicPortion {
        beam_id: verified_sender,
        cert: cert.clone(),
        pubkey: pubkey.into(),
    })
}

/// Verify whether the certificate is signed by root_ca_cert and the dates are valid
pub fn verify_cert(
    certificate: &X509,
    root_ca_cert: &X509,
) -> Result<(), CertificateInvalidReason> {
    let client_ok = certificate
        .verify(
            root_ca_cert
                .public_key()
                .map_err(|_e| CertificateInvalidReason::InvalidPublicKey)?
                .as_ref(),
        )
        .map_err(|e| CertificateInvalidReason::Other(e.to_string()))?;
    let date_ok =
        x509_date_valid(&certificate).map_err(|_err| CertificateInvalidReason::InvalidDate)?;

    match (client_ok, date_ok) {
        (true, true) => Ok(()), // TODO: Check if actually constant time
        (true, false) => Err(CertificateInvalidReason::InvalidDate),
        (false, _) => Err(CertificateInvalidReason::InvalidPublicKey),
    }
}

pub(crate) fn hash(data: &[u8]) -> Result<[u8; 32], SamplyBeamError> {
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let digest = hasher.finalize();
    let digest: [u8; 32] = digest[0..32].try_into().unwrap();
    Ok(digest)
}

pub fn get_own_privkey() -> &'static RsaPrivateKey {
    &config::CONFIG_SHARED_CRYPTO.get().unwrap().privkey_rsa
}
/* Utility Functions */

/// Extracts the pem-encoded public key from a x509 certificate
fn x509_cert_to_x509_public_key(cert: &X509) -> Result<Vec<u8>, SamplyBeamError> {
    match cert.public_key() {
        Ok(key) => key.public_key_to_pem().or_else(|_| {
            Err(SamplyBeamError::SignEncryptError(
                "Invalid public key in x509 certificate.".into(),
            ))
        }),
        Err(_) => Err(SamplyBeamError::SignEncryptError(
            "Unable to extract public key from certificate.".into(),
        )),
    }
}

/// Converts the x509 pem-encoded public key to the rsa public key
pub fn x509_public_key_to_rsa_pub_key(cert_key: &Vec<u8>) -> Result<RsaPublicKey, SamplyBeamError> {
    let rsa_key = RsaPublicKey::from_pkcs1_pem(std::str::from_utf8(cert_key).or_else(|e| {
        Err(SamplyBeamError::SignEncryptError(format!(
            "Invalid character in certificate public key because {}",
            e
        )))
    })?)
    .or_else(|e| {
        Err(SamplyBeamError::SignEncryptError(format!(
            "Can not extract public rsa key from certificate because {}",
            e
        )))
    });
    rsa_key
}

/// Convenience function to extract a rsa public key from a x509 certificate. Calls x509_cert_to_x509_public_key and x509_public_key_to_rsa_pub_key internally.
pub fn x509_cert_to_rsa_pub_key(cert: &X509) -> Result<RsaPublicKey, SamplyBeamError> {
    x509_public_key_to_rsa_pub_key(&x509_cert_to_x509_public_key(cert)?)
}

/// Converts the asn.1 time (e.g., from a certificate exiration date) to rust's SystemTime. From https://github.com/sfackler/rust-openssl/issues/1157#issuecomment-1016737160
pub fn asn1_time_to_system_time(time: &Asn1TimeRef) -> Result<SystemTime, ErrorStack> {
    let unix_time = Asn1Time::from_unix(0)?.diff(time)?;
    Ok(SystemTime::UNIX_EPOCH
        + Duration::from_secs(unix_time.days as u64 * 86400 + unix_time.secs as u64))
}

/// Checks if SystemTime::now() is between the not_before and the not_after dates of a x509 certificate
pub fn x509_date_valid(cert: &X509) -> Result<bool, ErrorStack> {
    let expirydate = asn1_time_to_system_time(cert.not_after())?;
    let startdate = asn1_time_to_system_time(cert.not_before())?;
    let now = SystemTime::now();
    return Ok(expirydate > now && now > startdate);
}

pub fn load_certificates_from_file(ca_file: PathBuf) -> Result<X509, SamplyBeamError> {
    let file = ca_file.as_path();
    let content = std::fs::read(file).map_err(|e| {
        SamplyBeamError::ConfigurationFailed(format!(
            "Can not load certificate {}: {}",
            file.to_string_lossy(),
            e
        ))
    })?;
    let cert = X509::from_pem(&content).map_err(|e| {
        SamplyBeamError::ConfigurationFailed(format!(
            "Unable to read certificate from file {}: {}",
            file.to_string_lossy(),
            e
        ))
    });
    cert
}

pub fn load_certificates_from_dir(ca_dir: Option<PathBuf>) -> Result<Vec<X509>, std::io::Error> {
    let mut result = Vec::new();
    if let Some(ca_dir) = ca_dir {
        for file in ca_dir.read_dir()? {
            //.map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Unable to read from TLS CA directory {}: {}", ca_dir.to_string_lossy(), e)))
            let path = file?.path();
            let content = std::fs::read(&path)?;
            let cert = X509::from_pem(&content);
            if let Err(e) = cert {
                warn!(
                    "Unable to read certificate from file {}: {}",
                    path.to_string_lossy(),
                    e
                );
                continue;
            }
            result.push(cert.unwrap());
        }
    }
    Ok(result)
}

/// Checks whether or not a x509 certificate matches a private key by comparing the (public) modulus
pub fn is_cert_from_privkey(cert: &X509, key: &RsaPrivateKey) -> Result<bool, ErrorStack> {
    let cert_rsa = cert.public_key()?.rsa()?;
    let cert_mod = cert_rsa.n();
    let key_mod = key.n();
    let key_mod_bignum = openssl::bn::BigNum::from_slice(&key_mod.to_bytes_be())?;
    let is_equal = cert_mod.ucmp(&key_mod_bignum) == std::cmp::Ordering::Equal;
    if !is_equal {
        match ProxyCertInfo::try_from(cert) {
            Ok(x) => {
                warn!(
                    "CA error: Found certificate (serial {}) that does not match private key.",
                    x.serial
                );
            }
            Err(_) => {
                warn!("CA error: Found a certificate that does not match private key; I cannot even parse it: {:?}", cert);
            }
        };
    }
    return Ok(is_equal);
}

/// Selects the newest certificate from a vector of certs by comparing the `not_before` time
pub fn get_newest_cert(certs: &mut Vec<CryptoPublicPortion>) -> Option<CryptoPublicPortion> {
    certs.sort_by(|a, b| {
        a.cert
            .not_before()
            .compare(b.cert.not_before())
            .expect("Unable to select newest certificate")
    }); // sort: newest last
    certs.pop() // return last (newest) certificate
}

/// Selecs the best fitting certificate from a vector of certs according to:
/// 1) Does it match the private key?
/// 2) Is the current date in the valid date range?
/// 3) Select the newest of the remaining
pub(crate) fn get_best_own_certificate(
    publics: impl Into<Vec<CryptoPublicPortion>>,
    private_rsa: &RsaPrivateKey,
) -> Option<CryptoPublicPortion> {
    let mut publics = publics.into();
    debug!(
        "get_best_certificate(): Considering {} certificates: {:?}",
        publics.len(),
        publics
    );
    publics.retain(|c| is_cert_from_privkey(&c.cert, private_rsa).unwrap_or(false)); // retain certs matching the private cert
    debug!(
        "get_best_certificate(): {} certificates match our private key.",
        publics.len()
    );
    publics.retain(|c| x509_date_valid(&c.cert).unwrap_or(false)); // retain certs with valid dates
    debug!(
        "get_best_certificate(): After sorting, {} certificates remaining.",
        publics.len()
    );
    get_newest_cert(&mut publics)
}

/// Selecs the best fitting certificate from a vector of certs according to:
/// 1) Is the current date in the valid date range?
/// 2) Select the newest of the remaining
pub fn get_best_other_certificate(
    publics: &Vec<CryptoPublicPortion>,
) -> Option<CryptoPublicPortion> {
    let mut publics = publics.to_owned();
    publics.retain(|c| x509_date_valid(&c.cert).unwrap_or(false)); // retain certs with valid dates
    get_newest_cert(&mut publics)
}

pub async fn get_proxy_public_keys(
    receivers: impl IntoIterator<Item = &AppOrProxyId>,
) -> Result<Vec<RsaPublicKey>, SamplyBeamError> {
    let proxy_receivers: Vec<ProxyId> = receivers
        .into_iter()
        .map(|app_or_proxy| match app_or_proxy {
            AppOrProxyId::ProxyId(id) => id.to_owned(),
            AppOrProxyId::AppId(id) => id.proxy_id(),
        })
        .collect();
    let receivers_crypto_bundle =
        crypto::get_newest_certs_for_cnames_as_pemstr(proxy_receivers.iter()).await;
    let receivers_keys = match receivers_crypto_bundle {
        Some(vec) => vec
            .iter()
            .map(|crypt_publ| {
                rsa::RsaPublicKey::from_public_key_pem(&crypt_publ.pubkey)
                    .expect("Cannot collect recipients' public keys")
            })
            .collect::<Vec<rsa::RsaPublicKey>>(), // TODO Expect
        None => Vec::new(),
    };
    Ok(receivers_keys)
}
