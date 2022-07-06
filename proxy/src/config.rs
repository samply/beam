use lazy_static::lazy_static;

use std::{net::SocketAddr, process::exit, collections::HashMap, fs::read_to_string, path::{Path, PathBuf}};

use argh::{FromArgs, FromArgValue};
use axum::http::HeaderValue;
use hyper::Uri;
use serde::Deserialize;
use shared::ClientId;
use tracing::{debug, info};

use shared::errors::SamplyBrokerError;

#[derive(Clone,Debug)]
pub struct Config {
    pub broker_uri: Uri,
    pub broker_host_header: HeaderValue,
    pub bind_addr: SocketAddr,
    pub pki_token: ApiKey,
    pub pki_realm: String,
    pub privkey_pem: String,
    pub client_id: ClientId
}

pub type ApiKey = String;

#[derive(Deserialize)]
struct SecretConfig {
    pki_token: String,
    apikeys: Vec<ApiKeyPair>
}

#[derive(Deserialize)]
struct ApiKeyPair {
    client: ClientId,
    key: ApiKey
}

/// Settings for Samply.Broker.Proxy
#[derive(FromArgs,Debug)]
pub struct CliArgs {
    /// local bind address (default: 0.0.0.0:8081)
    #[argh(option, default = "SocketAddr::from_arg_value(\"0.0.0.0:8081\").unwrap()")]
    pub bind_addr: SocketAddr,
    /// the broker's base URL, e.g. https://broker.samply.de
    #[argh(option)]
    pub broker_url: Uri,
    /// this proxy's client id
    #[argh(option)]
    pub client_id: String,
    /// samply.pki: URL to HTTPS endpoint
    #[argh(option)]
    pub pki_address: Uri,
    /// samply.pki: Authentication realm (default: samplypki)
    #[argh(option, default = "String::from(\"samplypki\")")]
    pub pki_realm: String,
    /// samply.pki: Path to file containing PKI secrets (default: /run/secrets/broker.secrets)
    #[argh(option, default = "Path::new(\"/run/secrets/broker.secrets\").to_owned()")]
    pub pki_secret_file: PathBuf,
    /// samply.pki: Path to own secret key (default: /run/secrets/privkey.pem)
    #[argh(option, default = "Path::new(\"/run/secrets/privkey.pem\").to_owned()")]
    pub privkey_file: PathBuf,
}

fn parse_secret_file(filename: &Path) -> Result<(String, HashMap<ApiKey,ClientId>),SamplyBrokerError>{
    let mut map = HashMap::new();
    let contents = read_to_string(filename)?;
    let secret_config = serde_json::from_str::<SecretConfig>(&contents)
        .map_err(|e| SamplyBrokerError::ReadSecretConfig(e.to_string()))?;
    
    for apikey in secret_config.apikeys {
        map.insert(apikey.key, apikey.client);
    }

    Ok((secret_config.pki_token, map))
}

pub(crate) fn get_config() -> Result<(Config,HashMap<ApiKey,ClientId>),SamplyBrokerError> {
    if std::env::args().len() <= 1 {
        eprintln!("Missing parameters, please run with --help.");
        exit(1);
    }
    let cli_args = argh::from_env::<CliArgs>();
    let (pki_token, api_keys) = parse_secret_file(&cli_args.pki_secret_file)?;
    let privkey_pem = read_to_string(cli_args.privkey_file)?;
    let client_id = ClientId::try_from(cli_args.client_id)
        .expect("Invalid Client ID supplied.");
    let config = Config {
        broker_host_header: uri_to_host_header(&cli_args.broker_url)?,
        broker_uri: cli_args.broker_url,
        pki_token,
        bind_addr: cli_args.bind_addr,
        pki_realm: cli_args.pki_realm,
        privkey_pem,
        client_id,
    };
    info!("Successfully read config and API keys from CLI and secrets file.");
    Ok((config, api_keys))
}

fn uri_to_host_header(uri: &Uri) -> Result<HeaderValue,SamplyBrokerError> {
    let hostname: String = uri.host()
        .ok_or(SamplyBrokerError::WrongBrokerUri("URI's host is empty."))?.into();
    let port = match uri.port() {
        Some(p) => format!(":{}",p),
        None => String::from(""),
    };
    let host_header = hostname + &port;
    let host_header: HeaderValue = HeaderValue::from_str(&host_header)
        .map_err(|_| SamplyBrokerError::WrongBrokerUri("Unable to parse broker URL"))?;
    Ok(host_header)
}

lazy_static!{
    pub static ref CONFIG: Config = {
        let (config, _) = get_config()
            .expect("Unable to read config");
        config
    };
}