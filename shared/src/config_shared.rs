use crate::{SamplyBeamError, crypto};
use dataobjects::beam_id::{BrokerId, BeamId};
use std::{path::PathBuf, rc::Rc, sync::Arc, fs::read_to_string};
use hyper::Uri;
use clap::Parser;
use jwt_simple::prelude::RS256KeyPair;
use rsa::{RsaPrivateKey, pkcs8::DecodePrivateKey, pkcs1::DecodeRsaPrivateKey};
use static_init::dynamic;

/// Settings for Samply.Beam (Shared)
#[derive(Parser,Debug)]
#[clap(author, version, about, long_about = None)]
struct CliArgs {
    /// Outgoing HTTP proxy (e.g. http://myproxy.mynetwork:3128)
    #[clap(long, env, value_parser)]
    pub http_proxy: Option<Uri>,

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
    // pub(crate) broker_url: Uri,
    pub(crate) broker_domain: String,
}

impl crate::config::Config for Config {
    fn load() -> Result<Self,SamplyBeamError> {
        let cli_args = CliArgs::parse();
        BrokerId::set_broker_id(cli_args.broker_url.host().unwrap().to_string());

        // Private key
        let privkey_pem = read_to_string(&cli_args.privkey_file)
            .map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Unable to load private key from file {}: {}", cli_args.privkey_file.to_string_lossy(), e)))?
            .trim().to_string();
        let privkey_rsa = RsaPrivateKey::from_pkcs1_pem(&privkey_pem)
            .or_else(|_| RsaPrivateKey::from_pkcs8_pem(&privkey_pem))
            .map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Unable to interpret private key PEM as PKCS#1 or PKCS#8: {}", e)))?;
        let mut privkey_rs256 = RS256KeyPair::from_pem(&privkey_pem)
            .map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Unable to interpret private key PEM as PKCS#1 or PKCS#8: {}", e)))?;
        if let Some(proxy_id) = cli_args.proxy_id {
            privkey_rs256 = privkey_rs256.with_key_id(&proxy_id);
        }
    
        // API Key
        let pki_apikey = read_to_string(cli_args.pki_apikey_file)
            .map_err(|_| SamplyBeamError::ConfigurationFailed("Failed to read PKI token.".into()))?
            .trim().to_string();

        let broker_domain = cli_args.broker_url.host();
        if false {
            todo!() // TODO Tobias: Check if matches certificate, and fail
        }
        let broker_domain = broker_domain.unwrap().to_string();
        Ok(Config { pki_address: cli_args.pki_address, pki_realm: cli_args.pki_realm, pki_apikey, privkey_rs256, privkey_rsa, http_proxy: cli_args.http_proxy, broker_domain })
    }    
}
