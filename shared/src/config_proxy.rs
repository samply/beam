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
    pub pki_realm: String,
    pub client_id: ClientId,
    pub api_keys: HashMap<ClientId,ApiKey>
}

pub type ApiKey = String;

/// Settings for Samply.Broker.Proxy
#[derive(Parser,Debug)]
#[clap(author, version, about, long_about = None, arg_required_else_help(true))]
pub struct CliArgs {
    /// Local bind address
    #[clap(long, env, value_parser, default_value_t = SocketAddr::from_str("0.0.0.0:8081").unwrap())]
    pub bind_addr: SocketAddr,

    /// Outgoing HTTP proxy (e.g. http://myproxy.mynetwork:3128)
    #[clap(long, env, value_parser)]
    pub http_proxy: Option<Uri>,
    
    /// The broker's base URL, e.g. https://broker.samply.de
    #[clap(long, env, value_parser)]
    pub broker_url: Uri,

    /// This proxy's client id, e.g. site23.broker.samply.de
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
            k.strip_prefix(CLIENT_KEY_PREFIX)
                .map(|stripped| (stripped.to_owned(), v))
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

impl crate::config::Config for Config {
    fn load() -> Result<Config,SamplyBrokerError> {
        let cli_args = CliArgs::parse();
        let client_id = ClientId::try_from(cli_args.client_id)
            .map_err(|_| SamplyBrokerError::ConfigurationFailed("Invalid Client ID supplied.".into()))?;
        let api_keys = parse_apikeys(&client_id)?;
        if api_keys.is_empty() {
            return Err(SamplyBrokerError::ConfigurationFailed(format!("No API keys have been defined. Please set environment vars à la {}<clientname>=<key>", CLIENT_KEY_PREFIX)));
        }
        let config = Config {
            broker_host_header: uri_to_host_header(&cli_args.broker_url)?,
            broker_uri: cli_args.broker_url,
            pki_address: cli_args.pki_address,
            bind_addr: cli_args.bind_addr,
            pki_realm: cli_args.pki_realm,
            client_id,
            api_keys
        };
        info!("Successfully read config and API keys from CLI and secrets file.");
        Ok(config)
    }        
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
