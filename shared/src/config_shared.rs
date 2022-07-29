use crate::{SamplyBeamError, crypto::{self, load_certificates_from_dir}, beam_id::{BrokerId, BeamId}};
use std::{path::PathBuf, rc::Rc, sync::Arc, fs::read_to_string};
use hyper::Uri;
use clap::Parser;
use hyper_tls::native_tls::Certificate;
use jwt_simple::prelude::RS256KeyPair;
use openssl::x509::X509;
use rsa::{RsaPrivateKey, pkcs8::DecodePrivateKey, pkcs1::DecodeRsaPrivateKey};
use static_init::dynamic;
use tracing::info;

#[derive(Parser,Debug)]
#[clap(name("Samply.Beam (shared library)"), version, arg_required_else_help(true))]
struct CliArgs {
    /// Outgoing HTTP proxy (e.g. http://myproxy.mynetwork:3128)
    #[clap(long, env, value_parser)]
    pub http_proxy: Option<String>,

    /// Outgoing HTTP proxy: Directory with CA certificates to trust for TLS connections (e.g. /etc/samply/cacerts/)
    #[clap(long, env, value_parser)]
    tls_ca_certificates_dir: Option<PathBuf>,

    /// samply.pki: URL to HTTPS endpoint
    #[clap(long, env, value_parser)]
    pki_address: Uri,

    /// samply.pki: Authentication realm
    #[clap(long, env, value_parser, default_value = "samply_pki")]
    pki_realm: String,

    /// samply.pki: File containing the authentication token
    #[clap(long, env, value_parser, default_value = "/run/secrets/pki.secret")]
    pki_apikey_file: PathBuf,

    /// samply.pki: Path to own secret key
    #[clap(long, env, value_parser, default_value = "/run/secrets/privkey.pem")]
    privkey_file: PathBuf,

    // TODO: The following arguments have been added for compatibility reasons with the proxy config. Find another way to merge configs.
    /// (included for technical reasons)
    #[clap(long, env, value_parser)]
    broker_url: Uri,

    /// (included for technical reasons)
    #[clap(long, env, value_parser)]
    proxy_id: Option<String>,

    /// (included for technical reasons)
    #[clap(action)]
    examples: Option<String>,

    /// (included for technical reasons)
    #[clap(long,hide(true))]
    test_threads: Option<String>
}

#[allow(dead_code)]
pub(crate) struct Config {
    pub(crate) pki_address: Uri,
    pub(crate) pki_realm: String,
    pub(crate) pki_apikey: String,
    pub(crate) privkey_rs256: RS256KeyPair,
    pub(crate) privkey_rsa: RsaPrivateKey,
    pub(crate) http_proxy: Option<Uri>,
    pub(crate) tls_ca_certificates_dir: Option<PathBuf>,
    // pub(crate) broker_url: Uri,
    pub(crate) broker_domain: String,
}

impl crate::config::Config for Config {
    fn load() -> Result<Self,SamplyBeamError> {
        let cli_args = CliArgs::parse();
        BrokerId::set_broker_id(cli_args.broker_url.host().unwrap().to_string());

        let (privkey_rsa, privkey_rs256) = load_crypto(&cli_args)?;
    
        // API Key
        let pki_apikey = read_to_string(cli_args.pki_apikey_file)
            .map_err(|_| SamplyBeamError::ConfigurationFailed("Failed to read PKI token.".into()))?
            .trim().to_string();

        let broker_domain = cli_args.broker_url.host();
        if false {
            todo!() // TODO Tobias: Check if matches certificate, and fail
        }
        let broker_domain = broker_domain.unwrap().to_string();
        let http_proxy: Option<Uri> = if let Some(proxy) = cli_args.http_proxy {
            if proxy.is_empty() {
                None
            } else {
                Some(proxy.parse()
                    .map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Not a valid proxy URL: {proxy}. Reason: {e}")))?)
            }
        } else { None };
        let tls_ca_certificates_dir = cli_args.tls_ca_certificates_dir;
        Ok(Config { pki_address: cli_args.pki_address, pki_realm: cli_args.pki_realm, pki_apikey, privkey_rs256, privkey_rsa, http_proxy, broker_domain, tls_ca_certificates_dir })
    }    
}

fn load_crypto(cli_args: &CliArgs) -> Result<(RsaPrivateKey, RS256KeyPair), SamplyBeamError> {
    let privkey_pem = read_to_string(&cli_args.privkey_file)
        .map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Unable to load private key from file {}: {}", cli_args.privkey_file.to_string_lossy(), e)))?
        .trim().to_string();
    let privkey_rsa = RsaPrivateKey::from_pkcs1_pem(&privkey_pem)
        .or_else(|_| RsaPrivateKey::from_pkcs8_pem(&privkey_pem))
        .map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Unable to interpret private key PEM as PKCS#1 or PKCS#8: {}", e)))?;
    let mut privkey_rs256 = RS256KeyPair::from_pem(&privkey_pem)
        .map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Unable to interpret private key PEM as PKCS#1 or PKCS#8: {}", e)))?;
    if let Some(proxy_id) = &cli_args.proxy_id {
        privkey_rs256 = privkey_rs256.with_key_id(proxy_id);
    }
    Ok((privkey_rsa, privkey_rs256))
}