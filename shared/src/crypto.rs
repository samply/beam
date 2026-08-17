use async_trait::async_trait;
use axum::{body::Body, http::Request, Json};

use itertools::Itertools;
use once_cell::sync::{Lazy, OnceCell};
use aws_lc_rs::{
    encoding::{AsDer, Pkcs8V1Der, PublicKeyX509Der},
    rsa::{KeyPair, PrivateDecryptingKey as RsaPrivateKey, PublicEncryptingKey as RsaPublicKey},
};
use sha2::{Digest, Sha256};
use std::{
    borrow::BorrowMut,
    collections::{HashMap, HashSet},
    error::Error,
    fs::read_to_string,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::{sync::{mpsc, oneshot, RwLock}, time::Instant};
use tracing::{debug, error, info, warn};
use x509_cert::{Certificate, crl::CertificateList, der::{Decode, DecodePem, Encode, EncodePem, pem::LineEnding}, ext::pkix::name::DirectoryString};
use x509_parser::parse_x509_certificate;

use beam_lib::{AppOrProxyId, ProxyId};
use crate::{
    crypto,
    errors::{CertificateInvalidReason, SamplyBeamError},
    EncryptedMsgTaskRequest, MsgTaskRequest,
};

type Serial = String;

fn decode_pem(input: &[u8]) -> Result<(String, Vec<u8>), SamplyBeamError> {
    x509_cert::der::pem::decode_vec(input)
        .map(|(label, der)| (label.into(), der))
        .map_err(|e| SamplyBeamError::SignEncryptError(e.to_string()))
}

#[derive(Clone)]
pub struct X509 {
    certificate: Arc<Certificate>,
    rsa_public_key: RsaPublicKey,
}

impl PartialEq for X509 { fn eq(&self, other: &Self) -> bool { self.certificate == other.certificate } }
impl Eq for X509 {}

impl std::fmt::Debug for X509 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cert = self.certificate.tbs_certificate();
        f.debug_struct("X509").field("subject", &cert.subject().to_string())
            .field("serial", &cert.serial_number().to_string()).finish()
    }
}

impl X509 {
    pub fn from_pem(input: &[u8]) -> Result<Self, SamplyBeamError> {
        let certificate = Certificate::from_pem(input).map_err(|e| SamplyBeamError::SignEncryptError(e.to_string()))?;
        let public_key = certificate.tbs_certificate().subject_public_key_info().to_der()
            .map_err(|e| SamplyBeamError::SignEncryptError(e.to_string()))?;
        let rsa_public_key = RsaPublicKey::from_der(&public_key)
            .map_err(|_| SamplyBeamError::SignEncryptError("Certificate does not contain an RSA public key".into()))?;
        Ok(Self { certificate: Arc::new(certificate), rsa_public_key })
    }

    fn public_key_der(&self) -> Result<Vec<u8>, SamplyBeamError> {
        self.certificate.tbs_certificate().subject_public_key_info().to_der()
            .map_err(|e| SamplyBeamError::SignEncryptError(e.to_string()))
    }

    pub fn raw_serial(&self) -> &[u8] {
        self.certificate.tbs_certificate().serial_number().as_bytes()
    }

    pub fn to_pem(&self) -> Result<Vec<u8>, SamplyBeamError> {
        self.certificate.to_pem(LineEnding::LF).map(String::into_bytes)
            .map_err(|e| SamplyBeamError::SignEncryptError(e.to_string()))
    }

    fn common_names(&self) -> impl Iterator<Item = String> + '_ {
        self.certificate.tbs_certificate().subject().iter()
            .filter(|a| a.oid == x509_cert::der::oid::db::rfc4519::COMMON_NAME)
            .filter_map(|a| DirectoryString::try_from(&a.value).ok())
            .map(|s| s.value().into_owned())
    }

    fn not_before_timestamp(&self) -> i64 {
        self.certificate.tbs_certificate().validity().not_before.to_unix_duration().as_secs() as i64
    }

    fn not_after_timestamp(&self) -> i64 {
        self.certificate.tbs_certificate().validity().not_after.to_unix_duration().as_secs() as i64
    }
}

#[derive(Clone, Debug)]
pub struct X509Crl(Arc<HashSet<Vec<u8>>>);

impl X509Crl {
    pub fn from_pem(input: &[u8]) -> Result<Self, SamplyBeamError> {
        let crl = CertificateList::from_pem(input).map_err(|e| SamplyBeamError::SignEncryptError(e.to_string()))?;
        Ok(Self::from_list(crl))
    }

    pub fn from_der(input: &[u8]) -> Result<Self, SamplyBeamError> {
        let crl = CertificateList::from_der(input).map_err(|e| SamplyBeamError::SignEncryptError(e.to_string()))?;
        Ok(Self::from_list(crl))
    }

    fn from_list(crl: CertificateList) -> Self {
        Self(crl.tbs_cert_list.revoked_certificates.unwrap_or_default().into_iter()
            .map(|r| r.serial_number.as_bytes().to_vec()).collect::<HashSet<_>>().into())
    }
}

pub struct ProxyCertInfo {
    pub proxy_name: String,
    pub valid_since: String,
    pub valid_until: String,
    pub common_name: String,
    pub serial: String,
}

impl TryFrom<&X509> for ProxyCertInfo {
    type Error = SamplyBeamError;

    fn try_from(cert: &X509) -> Result<Self, Self::Error> {
        let common_name = cert.common_names().next().ok_or(CertificateInvalidReason::NoCommonName)?;
        let validity = cert.certificate.tbs_certificate().validity();

        let certinfo = ProxyCertInfo {
            proxy_name: common_name
                .split('.')
                .next()
                .ok_or(CertificateInvalidReason::InvalidCommonName)?
                .into(),
            common_name,
            valid_since: validity.not_before.to_string(),
            valid_until: validity.not_after.to_string(),
            serial: cert.certificate.tbs_certificate().serial_number().to_string().replace(':', ""),
        };
        Ok(certinfo)
    }
}

#[derive(Clone, Debug)]
pub enum CertificateCacheEntry {
    Valid(X509),
    Invalid(CertificateInvalidReason),
}

#[derive(Debug, Clone, Copy)]
pub enum CertificateCacheUpdate {
    Updated(u32),
    UnChanged,
}

impl AsRef<u32> for CertificateCacheUpdate {
    fn as_ref(&self) -> &u32 {
        match self {
            CertificateCacheUpdate::Updated(i) => i,
            CertificateCacheUpdate::UnChanged => &0,
        }
    }
}

pub struct CertificateCache {
    serial_to_x509: HashMap<Serial, CertificateCacheEntry>,
    cn_to_serial: HashMap<ProxyId, Vec<Serial>>,
    update_trigger: mpsc::UnboundedSender<oneshot::Sender<Result<CertificateCacheUpdate, SamplyBeamError>>>,
    root_cert: Option<X509>, // Might not be available at initialization time
    im_cert: Option<X509>,   // Might not be available at initialization time
}

#[async_trait]
pub trait GetCerts: Sync + Send {
    async fn certificate_list_via_network(&self) -> Result<Vec<String>, SamplyBeamError>;
    async fn certificate_by_serial_as_pem(&self, serial: &str) -> Result<String, SamplyBeamError>;
    async fn im_certificate_as_pem(&self) -> Result<String, SamplyBeamError>;
    /// A callback that runs on a timer and returns if the cache changed
    async fn on_timer(&self, _cache: &mut CertificateCache) -> CertificateCacheUpdate { CertificateCacheUpdate::UnChanged }
    async fn get_crl(&self) -> Result<Option<X509Crl>, SamplyBeamError> { Ok(None) }
}

impl CertificateCache {
    pub fn new(
        update_trigger: mpsc::UnboundedSender<oneshot::Sender<Result<CertificateCacheUpdate, SamplyBeamError>>>,
    ) -> CertificateCache {
        Self {
            serial_to_x509: HashMap::new(),
            cn_to_serial: HashMap::new(),
            update_trigger,
            root_cert: None,
            im_cert: None,
        }
    }

    pub async fn wait_and_remove_oldest_cert(cache: Arc<RwLock<Self>>, abort_trigger: &mut mpsc::Receiver<()>) {
        // Get oldest cert, i.e. cert that will expire soonest
        let oldest_cert = {
            let cache_lock = cache.read().await;
            cache_lock
                .serial_to_x509
                .values()
                .filter_map(|entry| if let CertificateCacheEntry::Valid(cert) = entry {
                    Some(cert)
                } else {
                    None
                })
                .min_by_key(|cert| cert.not_after_timestamp())
                .cloned()
        };
        // if we dont have any certs yet, wait
        let Some(oldest_cert) = oldest_cert else {
            // An inefficient sleep is fine here as this will never be called once certs have been populated.
            tokio::time::sleep(Duration::from_secs(20)).await;
            return;
        };
        // Sleep until expired
        let expire_date = match timestamp_to_system_time(oldest_cert.not_after_timestamp()) {
            Ok(t) => t,
            Err(e) => {
                error!("Unable to read a certificate's expiration date: {:?}. Cert expiration will not work until this application is restarted. Offending certificate: {:?}", e, oldest_cert);
                return;
            }
        };
        let duration = expire_date.duration_since(SystemTime::now()).unwrap_or(Duration::from_secs(0)); // If 2 certs expire at the same time we want to expire them immediately
        let secs = duration.as_secs();
        info!("Oldest certificate will expire in: {}d {}h {}m {}s", secs / (24 * 60 * 60), (secs % (24 * 60 * 60)) / (60 * 60), (secs % (60 * 60)) / 60, secs % 60);
        let aborted = tokio::select! {
            _ = tokio::time::sleep(duration) => false,
            _ = abort_trigger.recv() => true
        };
        if aborted { 
            debug!("Aborted waiting for expiry of {oldest_cert:?}");
            return; 
        }
        info!("Invalidating old cert now: {:?}", oldest_cert);
        // Invalidate cert in cache
        {
            let mut cache_lock = cache.write().await;
            let Some(entry) = cache_lock
                .serial_to_x509
                .values_mut()
                .find(|other| if let CertificateCacheEntry::Valid(cert) = other {
                    cert == &oldest_cert
                } else {
                    false
                }
            ) else {
                error!("Unable to find expired certificate in our cache; this should not happen: {oldest_cert:?}. Cert expiration will not work until this application is restarted.");
                return;
            };
            *entry = CertificateCacheEntry::Invalid(CertificateInvalidReason::InvalidDate);
        }
    }

    /// Searches cache for a certificate with the given ClientId. If not found, updates cache from central vault. If then still not found, return None
    pub async fn get_all_certs_by_cname(cname: &ProxyId) -> Vec<CertificateCacheEntry> {
        // TODO: What if multiple certs are found?
        let mut result = get_all_certs_from_cache_by_cname(cname).await; // Drop Read Locks
        if result.iter().filter(|cert| matches!(cert, CertificateCacheEntry::Valid(_))).count() == 0 {
            
            // requires write lock.
            Self::update_certificates().await.unwrap_or_else(|e| {
                warn!("Updating certificates failed: {}", e);
                CertificateCacheUpdate::UnChanged
            });
            result = get_all_certs_from_cache_by_cname(cname).await;
            if result.is_empty() {
                warn!(
                    "Did not find certificate for cname {}, even after update.",
                    cname
                );
            } 
        } 
        result
    }

    /// Searches cache for a certificate with the given Serial. If not found, updates cache from central vault. If then still not found, return None
    pub async fn get_by_serial(serial: &str) -> Option<X509> {
        {
            // TODO: Do smart caching: Return reference to existing certificate that exists only once in memory.
            let cache = CERT_CACHE.read().await;
            if let Some(CertificateCacheEntry::Valid(cert)) = cache.serial_to_x509.get(serial) {
                return Some(cert.clone());
            }
        }
        Self::update_certificates().await.unwrap_or_else(|e| {
            // requires write lock.
            warn!("Updating certificates failed: {}", e);
            CertificateCacheUpdate::UnChanged
        });
        let cache = CERT_CACHE.read().await;
        match cache.serial_to_x509.get(serial)? {
            CertificateCacheEntry::Valid(v) => Some(v.clone()),
            CertificateCacheEntry::Invalid(e) => {
                warn!("Certificate with serial {serial} is invalid after update because of {e}");
                None
            }
        }
    }

    /// Manually update cache from fetching all certs from the central vault
    async fn update_certificates() -> Result<CertificateCacheUpdate, SamplyBeamError> {
        debug!("Triggering certificate update ...");
        let (tx, rx) = oneshot::channel();
        CERT_CACHE
            .read()
            .await
            .update_trigger
            .send(tx)
            .expect("Internal Error: Certificate Store Updater is not listening for requests.");
        debug!("Certificate update triggered -- waiting for results...");
        match rx.await {
            Ok(Ok(result)) => {
                debug!("Certificate update successfully completed: Got {} new certificates.", result.as_ref());
                Ok(result)
            }
            Ok(Err(e)) => {
                error!("Unable to sync certificates: {e}");
                Err(e)
            }
            Err(e) => {
                warn!("Unable to receive notification about certificate updates: {e}.");
                Err(SamplyBeamError::InternalSynchronizationError(e.to_string()))
            },
        }
    }

    fn invalidate_revoked_certs(&mut self, crl: &X509Crl) -> usize {
        let mut revoked_certs = 0;
        self.serial_to_x509.values_mut().for_each(|cert_entry| {
            if let CertificateCacheEntry::Valid(ref cert) = cert_entry {
                if is_revoked(crl, cert) {
                    *cert_entry = CertificateCacheEntry::Invalid(CertificateInvalidReason::Revoked);
                    revoked_certs += 1;
                }
            }
        });
        revoked_certs
    }

    pub async fn update_certificates_mut(
        &mut self,
    ) -> Result<CertificateCacheUpdate, SamplyBeamError> {
        debug!("Updating certificates via network ...");
        let certificate_list = CERT_GETTER
            .get()
            .unwrap()
            .certificate_list_via_network()
            .await?;
        let certificate_revocation_list = CERT_GETTER.get().unwrap().get_crl().await?;
        // Check if any of the certs in the cache have been revoked
        let mut revoked_certs = certificate_revocation_list
            .as_ref()
            .map(|crl| self.invalidate_revoked_certs(crl))
            .unwrap_or_default();
        debug!("Revoked {revoked_certs} certificates from cache.");
        let new_certificate_serials: Vec<&String> = certificate_list
            .iter()
            .filter(|serial| !self.serial_to_x509.contains_key(*serial))
            .collect();
        debug!(
            "Received {} certificates ({} of which were new).",
            certificate_list.len(),
            new_certificate_serials.len()
        );

        let mut new_count = 0;
        let cert_getter = CERT_GETTER.get().unwrap();
        let cert_pems = new_certificate_serials
            .iter()
            .map(|s| cert_getter.certificate_by_serial_as_pem(s))
            .collect::<futures::future::JoinAll<_>>()
            .await;
        for (serial, cert_pem) in new_certificate_serials.into_iter().zip(cert_pems) {
            debug!("Checking certificate with serial {serial}");

            let certificate = match cert_pem {
                Err(SamplyBeamError::CertificateError(err)) => {
                    debug!("Will skip invalid certificate {serial} from now on.");
                    self.serial_to_x509
                        .insert(serial.clone(), CertificateCacheEntry::Invalid(err));
                    continue;
                },
                Err(other_error) => {
                    warn!("Could not retrieve certificate for serial {serial}: {other_error}");
                    continue;
                }
                Ok(cert) => cert,
            };
            let parsed_cert = match X509::from_pem(certificate.as_bytes()) {
                Ok(x) => x,
                Err(err) => {
                    error!("Skipping unparsable certificate {serial}: {err}");
                    continue;
                }
            };
            // Check if the new cert is already revoked
            if certificate_revocation_list.as_ref().is_some_and(|list| is_revoked(list, &parsed_cert)) {
                self.serial_to_x509.insert(serial.clone(), CertificateCacheEntry::Invalid(CertificateInvalidReason::Revoked));
                revoked_certs += 1;
                continue;
            };
            let commonnames: Vec<ProxyId> = parsed_cert
                .common_names()
                .flat_map(|x| ProxyId::new(x).map_err(|e| warn!("Internal error: Vault returned certificate with invalid common name: {e}")))
                .collect();

            let err = {
                if commonnames.is_empty() {
                    Some(CertificateInvalidReason::NoCommonName)
                } else if let Err(e) = verify_cert(
                    &parsed_cert,
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
                    .insert(serial.clone(), CertificateCacheEntry::Valid(parsed_cert));
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
        if revoked_certs == 0 && new_count == 0 {
            Ok(CertificateCacheUpdate::UnChanged)
        } else {
            Ok(CertificateCacheUpdate::Updated(new_count))
        }
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
        let im_cert = match get_im_cert().await {
            Ok(im_cert) => X509::from_pem(im_cert.as_bytes())?,
            Err(SamplyBeamError::BrokerAuthorizationFailed) => {
                error!("Unable to retrieve intermediate CA certificate from broker.");
                error!("This means that either:");
                error!("    1. The CSR has not yet been signed by the central PKI");
                error!("    2. This Beam Proxy's certificate has expired and needs to be resigned in the central PKI.");
                std::process::exit(17);
            }
            Err(e) => return Err(e),
        };
        let root_cert = self
            .root_cert
            .as_ref()
            .expect("No root certificate set!");
        if let Err(e) = verify_cert(&im_cert, &root_cert) {
            error!("Intermediate certificate is invalid: {e:#}");
            error!(
                "Please send this info to the central beam admin for debugging:\n---BEGIN DEBUG---\n{}\nroot\n{}\n---END DEBUG---",
                format!("{im_cert:?}"),
                format!("{root_cert:?}")
            );
            std::process::exit(1);
        }
        self.im_cert = Some(im_cert);
        Ok(())
    }
}

async fn get_all_certs_from_cache_by_cname(cname: &ProxyId) -> Vec<CertificateCacheEntry> {
    let mut result = Vec::new();
        
    debug!("Getting cert(s) with cname {}", cname);
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
                        CertificateCacheEntry::Invalid(_reason) => {
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
                            }
                        }
                    }
                }
            }
        };
    }
    debug!(
        "Found {} valid and {} invalid certificate(s) for cname {} in cache.",
        result.len(),
        invalid,
        cname
    );
    result
}

/// Wrapper for initializing the CA chain. Must be called *after* config initialization
pub async fn init_ca_chain(root_cert: &X509) -> Result<(), SamplyBeamError> {
    let mut cache = CERT_CACHE.write().await;
    cache.set_root_cert(root_cert);
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

pub async fn get_serial_list() -> Vec<String> {
    let cache = CERT_CACHE.read().await;
    cache.serial_to_x509.iter()
        .filter(|(_, v)| matches!(v, CertificateCacheEntry::Valid(_)))
        .map(|(k, _)| k)
        .cloned()
        .collect()
}

pub async fn get_im_cert() -> Result<String, SamplyBeamError> {
    CERT_GETTER.get().unwrap().im_certificate_as_pem().await
}

pub(crate) static CERT_CACHE: Lazy<Arc<RwLock<CertificateCache>>> = Lazy::new(|| {
    let (tx_refresh, mut rx_refresh) = mpsc::unbounded_channel::<oneshot::Sender<Result<CertificateCacheUpdate, SamplyBeamError>>>();
    let (tx_newcerts, mut rx_newcerts) = mpsc::channel::<()>(1);
    let cc = Arc::new(RwLock::new(CertificateCache::new(tx_refresh)));
    let cc2 = cc.clone();
    let cc3: Arc<RwLock<CertificateCache>> = cc.clone();
    tokio::task::spawn(async move {
        loop {
            let sender = tokio::select! {
                Some(sender) = rx_refresh.recv() => {
                    debug!("Certificate cache refresh triggered by another component.");
                    Some(sender)
                },
                _ = tokio::time::sleep(Duration::from_secs(60)) => {
                    debug!("Certificate cache refresh after 60 seconds ...");
                    None
                }
            };
            let started = Instant::now();
            let mut locked_cache = cc2.write().await;
            let update;
            // Cache update from by a function
            if let Some(sender) = sender {
                let result = locked_cache.update_certificates_mut().await;
                update = *result.as_ref().unwrap_or(&CertificateCacheUpdate::UnChanged);
                if let Err(_err) = sender.send(result) {
                    warn!("Unable to inform requesting thread that CertificateCache has been updated. Maybe it stopped?");
                }
            // Cache update on a timer
            } else {
                // Note: This currently only updates the Cache on the broker as the default implementation of `GetCerts` does no update the cache 
                update = CERT_GETTER.get().unwrap().on_timer(&mut locked_cache).await;
            }
            if let CertificateCacheUpdate::Updated(count) = update {
                info!("Added {count} new certificates.");
                if let Err(e) = tx_newcerts.send(()).await {
                    warn!("Unable to inform cert expirer about a newly arrived certificate. Err: {e}. Continuing.");
                }
            }
            let elapsed = Instant::now() - started;
            const FIVE_SECS: Duration = Duration::from_secs(5);
            if elapsed > FIVE_SECS {
                warn!("Certificate update request took {} seconds.", elapsed.as_secs());
            } else {
                debug!("Certificate update request took {} seconds.", elapsed.as_secs());
            }
        }
    });
    tokio::spawn(async move {
        loop {
            CertificateCache::wait_and_remove_oldest_cert(cc3.clone(), &mut rx_newcerts).await;
        }
    });
    cc
});

async fn get_all_certs_by_cname(cname: &ProxyId) -> Vec<CertificateCacheEntry> {
    CertificateCache::get_all_certs_by_cname(cname).await
}

#[derive(Debug, Clone)]
pub struct CryptoPublicPortion {
    pub beam_id: ProxyId,
    pub cert: X509,
    pub rsa_public_key: RsaPublicKey,
    pub jwt_public_key_der: Vec<u8>,
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
    CertificateCache::get_by_serial(serial).await.as_ref().map(extract_x509)
}

pub async fn get_newest_certs_for_cnames_as_pemstr(
    cnames: Vec<ProxyId>,
) -> Vec<Result<CryptoPublicPortion, ProxyId>> {
    let mut result = Vec::with_capacity(cnames.len()); // No fancy map/iter, bc of async
    for id in cnames {
        let certs = get_all_certs_and_clients_by_cname_as_pemstr(&id)
            .await
            .into_iter()
            .flatten()
            .collect();
        if let Some(best_candidate) = get_best_other_certificate(&certs) {
            result.push(Ok(best_candidate));
        } else {
            result.push(Err(id))
        }
    }
    result
}

fn extract_x509(cert: &X509) -> Result<CryptoPublicPortion, CertificateInvalidReason> {
    let common_name = cert.common_names().next().ok_or(CertificateInvalidReason::NoCommonName)?;
    let verified_sender = ProxyId::new(common_name)
        .map_err(|_| CertificateInvalidReason::InvalidCommonName)?;
    Ok(CryptoPublicPortion {
        beam_id: verified_sender,
        cert: cert.clone(),
        rsa_public_key: cert.rsa_public_key.clone(),
        jwt_public_key_der: cert.certificate.tbs_certificate().subject_public_key_info().subject_public_key.raw_bytes().to_vec(),
    })
}

/// Verify whether the certificate is signed by root_ca_cert and the dates are valid
pub fn verify_cert(
    certificate: &X509,
    root_ca_cert: &X509,
) -> Result<(), CertificateInvalidReason> {
    let certificate_der = certificate.certificate.to_der().map_err(|e| CertificateInvalidReason::Other(e.to_string()))?;
    let issuer_der = root_ca_cert.certificate.to_der().map_err(|e| CertificateInvalidReason::Other(e.to_string()))?;
    let (_, certificate) = parse_x509_certificate(&certificate_der).map_err(|e| CertificateInvalidReason::Other(e.to_string()))?;
    let (_, issuer) = parse_x509_certificate(&issuer_der).map_err(|e| CertificateInvalidReason::Other(e.to_string()))?;
    certificate.verify_signature(Some(issuer.public_key()))
        .map_err(|_| CertificateInvalidReason::InvalidPublicKey)?;
    if certificate.validity().is_valid() {
        Ok(())
    } else {
        Err(CertificateInvalidReason::InvalidDate)
    }
}

pub(crate) fn hash(data: &[u8]) -> Result<[u8; 32], SamplyBeamError> {
    let mut hasher = Sha256::new();
    hasher.update(&data);
    let digest = hasher.finalize();
    let digest: [u8; 32] = digest[0..32].try_into().unwrap();
    Ok(digest)
}

/* Utility Functions */

/// Extracts the pem-encoded public key from a x509 certificate
pub fn x509_cert_to_x509_public_key(cert: &X509) -> Result<Vec<u8>, SamplyBeamError> {
    cert.public_key_der()
}

/// Converts the x509 pem-encoded public key to the rsa public key
pub fn x509_public_key_to_rsa_pub_key(cert_key: &Vec<u8>) -> Result<RsaPublicKey, SamplyBeamError> {
    RsaPublicKey::from_der(cert_key).map_err(|e| SamplyBeamError::SignEncryptError(format!(
        "Can not extract public RSA key from certificate: {e}"
    )))
}

/// Convenience function to extract a rsa public key from a x509 certificate. Calls x509_cert_to_x509_public_key and x509_public_key_to_rsa_pub_key internally.
pub fn x509_cert_to_rsa_pub_key(cert: &X509) -> Result<RsaPublicKey, SamplyBeamError> {
    x509_public_key_to_rsa_pub_key(&x509_cert_to_x509_public_key(cert)?)
}

pub fn rsa_private_key_from_pem(input: &[u8]) -> Result<RsaPrivateKey, SamplyBeamError> {
    let (label, der) = decode_pem(input)?;
    match label.as_str() {
        "PRIVATE KEY" => RsaPrivateKey::from_pkcs8(&der)
            .map_err(|e| SamplyBeamError::SignEncryptError(e.to_string())),
        "RSA PRIVATE KEY" => {
            let key_pair = KeyPair::from_der(&der)
                .map_err(|e| SamplyBeamError::SignEncryptError(e.to_string()))?;
            let pkcs8 = AsDer::<Pkcs8V1Der>::as_der(&key_pair)
                .map_err(|_| SamplyBeamError::SignEncryptError("Unable to convert PKCS#1 RSA key to PKCS#8".into()))?;
            RsaPrivateKey::from_pkcs8(pkcs8.as_ref())
                .map_err(|e| SamplyBeamError::SignEncryptError(e.to_string()))
        }
        tag => Err(SamplyBeamError::SignEncryptError(format!("Unsupported private key PEM tag: {tag}"))),
    }
}

/// Converts the asn.1 time (e.g., from a certificate exiration date) to rust's SystemTime. From https://github.com/sfackler/rust-openssl/issues/1157#issuecomment-1016737160
pub fn timestamp_to_system_time(timestamp: i64) -> Result<SystemTime, SamplyBeamError> {
    u64::try_from(timestamp)
        .map(|seconds| SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
        .map_err(|_| SamplyBeamError::SignEncryptError("Certificate timestamp predates UNIX epoch".into()))
}

pub fn asn_str_to_vault_str(serial: &[u8]) -> Result<String, SamplyBeamError> {
    Ok(serial.iter().map(|byte| format!("{byte:02x}")).join(":"))
}

/// Checks if SystemTime::now() is between the not_before and the not_after dates of a x509 certificate
pub fn x509_date_valid(cert: &X509) -> Result<bool, SamplyBeamError> {
    let validity = cert.certificate.tbs_certificate().validity();
    let now = SystemTime::now();
    Ok(validity.not_before.to_system_time() <= now && now <= validity.not_after.to_system_time())
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

pub fn load_certificates_from_dir(ca_dir: Option<PathBuf>) -> Result<Vec<reqwest::Certificate>, std::io::Error> {
    let mut result = Vec::new();
    if let Some(ca_dir) = ca_dir {
        for file in ca_dir.read_dir()? {
            //.map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Unable to read from TLS CA directory {}: {}", ca_dir.to_string_lossy(), e)))
            let path = file?.path();
            let content = std::fs::read(&path)?;
            let cert = reqwest::Certificate::from_pem(&content);
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
pub fn is_cert_from_privkey(cert: &X509, key: &RsaPrivateKey) -> Result<bool, SamplyBeamError> {
    let key_der = AsDer::<PublicKeyX509Der>::as_der(&key.public_key())
        .map_err(|_| SamplyBeamError::SignEncryptError("Unable to encode public key".into()))?;
    let is_equal = cert.public_key_der()?.as_slice() == key_der.as_ref();
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

pub fn parse_crl(der: &[u8]) -> Result<X509Crl, SamplyBeamError> {
    X509Crl::from_der(der)
}

fn is_revoked(crl: &X509Crl, cert: &X509) -> bool {
    crl.0.contains(cert.raw_serial())
}

/// Selects the newest certificate from a vector of certs by comparing the `not_before` time
pub fn get_newest_cert(certs: &mut Vec<CryptoPublicPortion>) -> Option<CryptoPublicPortion> {
    certs.sort_by(|a, b| {
        a.cert.not_before_timestamp().cmp(&b.cert.not_before_timestamp())
    }); // sort: newest last
    certs.pop() // return last (newest) certificate
}

/// Selecs the best fitting certificate from a vector of certs according to:
/// 1) Does it match the private key?
/// 2) Is the current date in the valid date range?
/// 3) Select the newest of the remaining
pub fn get_best_own_certificate(
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
        .map(|app_or_proxy| app_or_proxy.proxy_id())
        .collect();
    let receivers_crypto_bundle =
        crypto::get_newest_certs_for_cnames_as_pemstr(proxy_receivers).await;
    let (receivers_keys, proxies_with_invalid_certs): (Vec<_>, Vec<_>) = receivers_crypto_bundle
        .into_iter()
        .map(|crypt_publ_res| crypt_publ_res.map(|crypto| crypto.rsa_public_key))
        .partition_result();
    if proxies_with_invalid_certs.is_empty() {
        Ok(receivers_keys)
    } else {
        Err(SamplyBeamError::InvalidReceivers(proxies_with_invalid_certs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_str() {
        let input = [0x44, 0x0e, 0x0d, 0x94, 0xf3, 0x69, 0x66, 0x39, 0x11, 0x17, 0xbc, 0x9f, 0x86, 0x7d, 0x84, 0xf0, 0xc4, 0x8c, 0xfc, 0xb7];
        let expected = "44:0e:0d:94:f3:69:66:39:11:17:bc:9f:86:7d:84:f0:c4:8c:fc:b7";
        assert_eq!(expected, asn_str_to_vault_str(&input).unwrap());
    }

    const CERT_TO_REVOKE: &[u8] = b"-----BEGIN CERTIFICATE-----\nMIIDLjCCAhYCFCNuyAi2zfAyORDDiwsJnfJojBk8MA0GCSqGSIb3DQEBCwUAMFQx\nCzAJBgNVBAYTAkRFMRMwEQYDVQQIDApIZWlkZWxiZXJnMSEwHwYDVQQKDBhJbnRl\ncm5ldCBXaWRnaXRzIFB0eSBMdGQxDTALBgNVBAMMBHRlc3QwHhcNMjMwODI0MDc1\nMjM1WhcNMjMwOTIzMDc1MjM1WjBTMQswCQYDVQQGEwJERTETMBEGA1UECAwKU29t\nZS1TdGF0ZTEhMB8GA1UECgwYSW50ZXJuZXQgV2lkZ2l0cyBQdHkgTHRkMQwwCgYD\nVQQDDANmb28wggEiMA0GCSqGSIb3DQEBAQUAA4IBDwAwggEKAoIBAQDd2aLmn3EX\nkSMIxdMWXe8oQNyWBktyBoNK+gSyYBO3SkIcRKM41Ama4GgeIJnDRbL2XLC3Gkhv\nHyvBocVYeP/kWtw8Zvmmi/9Ztv04pVn6LzX2Yaqtm9X78Jo3n2ug2cC8IEoMaYbF\nTcUuV7IX1oSF4Fo3KRRoAUki6yok3uEFVH5cl/UPYyYRJ+CKvoras4c9arZ3Nk3G\na9ImlniBPZ3qQwnkJX5pKcKFzYka7xrNbCpInF/v68R9Hiy4YwUQbGeTfTM+W3i9\nn5ZnSWuwY5lew3WSnpcfYKJQCLhJ9iAXq13+oYbDFSA12pSBIEz0xve3/zR5Cg81\nLGKtvpllzfGVAgMBAAEwDQYJKoZIhvcNAQELBQADggEBAGk2Zii31WPqXwzAUNc0\nS6GjTkHMP5gzdTjYspdBOm8bdJROEp9O/vjAc2Oci4waI9FT6oZPhwX/a6TDtUGs\nAZeQYt9vlS4LPgs6RTF4sFXy+pl7EA/wYqb7e0LSVsx7feTpeRRCIbFXenTKa7m+\nMXsDRCR9weplJdFeyBodFBsNMpShOe3WbnQ7Gi3jLYCUb7acX4I4H4VA7HdakZJr\nEJzP0TQzt/vrSwA2GsNWgO5sOXYkYvjieqzfi89fqY6ZT2jWQ+v+wc7kDiBRbkVU\nGooK1Vo2TJYeaPPmyNomRZtlpgXBGztYyJTfPY0A0M1Fky8Y8QLObtxG0/fkWOft\nHyU=\n-----END CERTIFICATE-----";
    const CRL: &[u8] = b"-----BEGIN X509 CRL-----\nMIIB1zCBwAIBATANBgkqhkiG9w0BAQsFADBUMQswCQYDVQQGEwJERTETMBEGA1UE\nCAwKSGVpZGVsYmVyZzEhMB8GA1UECgwYSW50ZXJuZXQgV2lkZ2l0cyBQdHkgTHRk\nMQ0wCwYDVQQDDAR0ZXN0Fw0yMzA4MjQwODM3MTRaFw0yMzA5MjMwODM3MTRaMCcw\nJQIUI27ICLbN8DI5EMOLCwmd8miMGTwXDTIzMDgyNDA4MzQxNlqgDzANMAsGA1Ud\nFAQEAgIQADANBgkqhkiG9w0BAQsFAAOCAQEAJCLrxzeDdgRIqfGEPjBff21Tefir\n3mbxZtrCa232zJLmurX1zQ5S9pa/QvGQ/Fj91FUbNezomh1NTmJkscj3Mh8Ph/Mv\nIbburXhPG5ypHeOXAGQqpKADZyBPMRwIWaTqmtsMg5kdHzYScvvHFZRcy8KCKx6e\niFdqNc9qZkyvCazpzjWK+JpK6TPCpI68LO/DxhWPirclhjZLs3z6iAuxmW8TM71T\nC7YzZ0Z17xCttNW7155LpFWUo1YOQk1Cy9W2d3EIBMmZhn6yBUExusXzcj4BnXZ7\nzCqIhPnMU4nLrarkzgmy+v1ysdo1lFGQ4fC3XFY+oWxUsImFP9JKHKEbBA==\n-----END X509 CRL-----";

    #[test]
    fn test_revokation() {
        let mut cache = CertificateCache::new(mpsc::unbounded_channel().0);
        cache.serial_to_x509.insert("revoked".into(), CertificateCacheEntry::Valid(X509::from_pem(CERT_TO_REVOKE).unwrap()));
        let crl = X509Crl::from_pem(CRL).unwrap();
        cache.invalidate_revoked_certs(&crl);
        
        assert!(matches!(cache.serial_to_x509.get("revoked"), Some(&CertificateCacheEntry::Invalid(CertificateInvalidReason::Revoked))), "Certificate was not revoked");
    }
}
