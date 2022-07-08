use crate::{SamplyBrokerError, crypto};
use std::{path::PathBuf, rc::Rc, sync::Arc, fs::read_to_string};
use hyper::Uri;
use clap::Parser;
use jwt_simple::prelude::RS256KeyPair;
use lazy_static::lazy_static;
use rsa::{RsaPrivateKey, pkcs8::FromPrivateKey, pkcs1::FromRsaPrivateKey};

/// Settings for Samply.Broker.Shared
#[derive(Parser,Debug)]
#[clap(author, version, about, long_about = None)]
struct VaultConfig {
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
    broker_url: Option<Uri>,

    /// (included for technical reasons)
    #[clap(long, env, value_parser)]
    client_id: Option<String>,
}

pub(crate) struct Config {
    pub(crate) pki_address: Uri,
    pub(crate) pki_realm: String,
    pub(crate) pki_apikey: String,
    // pub(crate) privkey_pem: String,
    pub(crate) privkey_rs256: RS256KeyPair,
    pub(crate) privkey_rsa: RsaPrivateKey
}

pub(crate) struct Keys {
    pub(crate) my_rs256: RS256KeyPair,
}

lazy_static!{
    pub(crate) static ref CONFIG: Config = {
        let vc = VaultConfig::parse();
        let privkey_pem = read_to_string(&vc.privkey_file)
            .expect("Unable to load private key")
            .trim().to_string();
        let privkey_rsa = RsaPrivateKey::from_pkcs1_pem(&privkey_pem)
            .or(RsaPrivateKey::from_pkcs8_pem(&privkey_pem))
            .expect("Unable to interpret private key PEM as PKCS#1 or PKCS#8.");
        let pki_apikey = read_to_string(vc.pki_apikey_file)
            .expect("Failed to read PKI token.")
            .trim().to_string();
        let mut privkey_rs256 = RS256KeyPair::from_pem(&privkey_pem)
            .expect("Failed to read PKI token.");
        if let Some(client_id) = vc.client_id {
            privkey_rs256 = privkey_rs256.with_key_id(&client_id);
        }
        Config { pki_address: vc.pki_address, pki_realm: vc.pki_realm, pki_apikey, privkey_rs256, privkey_rsa }
    };
}