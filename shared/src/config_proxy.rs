use clap::Parser;

use std::{net::SocketAddr, process::exit, collections::HashMap, fs::read_to_string, str::FromStr, path::{Path, PathBuf}};

use axum::http::HeaderValue;
use hyper::Uri;
use serde::Deserialize;
use crate::ClientId;
use tracing::{info, debug};

use crate::errors::SamplyBrokerError;

#[derive(Clone,Debug)]
pub struct Config {
    pub broker_uri: Uri,
    pub broker_host_header: HeaderValue,
    pub bind_addr: SocketAddr,
    pub pki_address: Uri,
    pub pki_token: ApiKey,
    pub pki_realm: String,
    pub privkey_pem: String,
    pub client_id: ClientId,
    pub api_keys: HashMap<ClientId,ApiKey>
}

pub type ApiKey = String;

/// Settings for Samply.Broker.Proxy
#[derive(Parser,Debug)]
#[clap(author, version, about, long_about = None, arg_required_else_help(true))]
pub struct CliArgs {
    /// local bind address
    #[clap(long, env, value_parser, default_value_t = SocketAddr::from_str("0.0.0.0:8081").unwrap())]
    pub bind_addr: SocketAddr,
    
    /// the broker's base URL, e.g. https://broker.samply.de
    #[clap(long, env, value_parser)]
    pub broker_url: Uri,

    /// this proxy's client id, e.g. site23.broker.samply.de
    #[clap(long, env, value_parser)]
    pub client_id: String,

    /// samply.pki: URL to HTTPS endpoint
    #[clap(long, env, value_parser)]
    pub pki_address: Uri,

    /// samply.pki: Authentication realm
    #[clap(long, env, value_parser, default_value = "samply_pki")]
    pub pki_realm: String,

    /// samply.pki: File containing the authentication token
    #[clap(long, env, value_parser, default_value = "/run/secrets/pki.secret")]
    pub pki_apikey_file: PathBuf,

    /// samply.pki: Path to own secret key
    #[clap(long, env, value_parser, default_value = "/run/secrets/privkey.pem")]
    pub privkey_file: PathBuf,
}

pub const CLIENT_KEY_PREFIX: &str = "CLIENTKEY_";

fn parse_apikeys(client_id: &ClientId) -> Result<HashMap<ClientId,ApiKey>,SamplyBrokerError>{
    std::env::vars()
        .filter_map(|(k,v)| {
            match k.strip_prefix(CLIENT_KEY_PREFIX) {
                Some(stripped) => Some((stripped.to_owned(), v)),
                None => None,
            }
        })
        .map(|(stripped,v)| {
            let client_id = format!("{}.{}", stripped, client_id);
            let client_id = ClientId::new(&client_id)
                .map_err(|_| SamplyBrokerError::ConfigurationFailed(format!("Wrong api key definition: Client ID {} is invalid.", client_id)))?;
            if v.is_empty() {
                return Err(SamplyBrokerError::ConfigurationFailed(format!("Unable to assign empty API key for client {}", client_id)));
            }
            Ok((client_id, v))
        })
        .collect()
}

pub(crate) fn get_config() -> Result<Config,SamplyBrokerError> {
    let cli_args = CliArgs::parse();
    let privkey_pem = read_to_string(cli_args.privkey_file)?.trim().to_string();
    let pki_token = read_to_string(cli_args.pki_apikey_file)?.trim().to_string();
    let client_id = ClientId::try_from(cli_args.client_id)
        .expect("Invalid Client ID supplied.");
    let api_keys = parse_apikeys(&client_id)?;
    if api_keys.is_empty() {
        return Err(SamplyBrokerError::ConfigurationFailed(format!("No API keys have been defined. Please set environment vars à la {}<clientname>=<key>", CLIENT_KEY_PREFIX)));
    }
    let config = Config {
        broker_host_header: uri_to_host_header(&cli_args.broker_url)?,
        broker_uri: cli_args.broker_url,
        pki_address: cli_args.pki_address,
        pki_token,
        bind_addr: cli_args.bind_addr,
        pki_realm: cli_args.pki_realm,
        privkey_pem,
        client_id,
        api_keys
    };
    info!("Successfully read config and API keys from CLI and secrets file.");
    Ok(config)
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
