use clap::Parser;

use std::{net::SocketAddr, process::exit, collections::HashMap, fs::read_to_string, str::FromStr, path::{Path, PathBuf}};

use axum::http::HeaderValue;
use hyper::Uri;
use serde::Deserialize;
use tracing::{info, debug};

use crate::errors::SamplyBeamError;
use dataobjects::beam_id::{AppId, BeamId, ProxyId, self, BrokerId};

#[derive(Clone,Debug)]
pub struct Config {
    pub broker_uri: Uri,
    pub broker_host_header: HeaderValue,
    pub bind_addr: SocketAddr,
    pub pki_address: Uri,
    pub pki_realm: String,
    pub proxy_id: ProxyId,
    pub api_keys: HashMap<AppId,ApiKey>
}

pub type ApiKey = String;

/// Settings for Samply.Beam (Proxy)
#[derive(Parser,Debug)]
#[clap(author, version, about, long_about = None, arg_required_else_help(true))]
pub struct CliArgs {
    /// Local bind address
    #[clap(long, env, value_parser, default_value_t = SocketAddr::from_str("0.0.0.0:8081").unwrap())]
    pub bind_addr: SocketAddr,

    /// Outgoing HTTP proxy (e.g. http://myproxy.mynetwork:3128)
    #[clap(long, env, value_parser)]
    pub http_proxy: Option<Uri>,
    
    /// The broker's base URL, e.g. https://broker23.beam.samply.de
    #[clap(long, env, value_parser)]
    pub broker_url: Uri,

    /// This proxy's beam id, e.g. proxy42.broker23.beam.samply.de
    #[clap(long, env, value_parser)]
    pub proxy_id: String,

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

    /// (included for technical reasons)
    #[clap(long,hide(true))]
    test_threads: Option<String>
}

pub const APPKEY_PREFIX: &str = "APPKEY_";

fn parse_apikeys(proxy_id: &ProxyId) -> Result<HashMap<AppId,ApiKey>,SamplyBeamError>{
    std::env::vars()
        .filter_map(|(k,v)| {
            k.strip_prefix(APPKEY_PREFIX)
                .map(|stripped| (stripped.to_owned(), v))
        })
        .map(|(stripped,v)| {
            let app_id = format!("{}.{}", stripped, proxy_id);
            let app_id = AppId::new(&app_id)
                .map_err(|_| SamplyBeamError::ConfigurationFailed(format!("Faulty API key definition: Resulting App ID  {} is invalid.", app_id)))?;
            if v.is_empty() {
                return Err(SamplyBeamError::ConfigurationFailed(format!("Unable to assign empty API key for client {}", app_id)));
            }
            Ok((app_id, v))
        })
        .collect()
}

impl crate::config::Config for Config {
    fn load() -> Result<Config,SamplyBeamError> {
        let cli_args = CliArgs::parse();
        BrokerId::set_broker_id(cli_args.broker_url.host().unwrap().to_string());
        let proxy_id = ProxyId::new(&cli_args.proxy_id)
            .map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Invalid Beam ID \"{}\" supplied: {}", cli_args.proxy_id, e)))?;
        let api_keys = parse_apikeys(&proxy_id)?;
        if api_keys.is_empty() {
            return Err(SamplyBeamError::ConfigurationFailed(format!("No API keys have been defined. Please set environment vars à la {}<clientname>=<key>", APPKEY_PREFIX)));
        }
        let config = Config {
            broker_host_header: uri_to_host_header(&cli_args.broker_url)?,
            broker_uri: cli_args.broker_url,
            pki_address: cli_args.pki_address,
            bind_addr: cli_args.bind_addr,
            pki_realm: cli_args.pki_realm,
            proxy_id,
            api_keys
        };
        info!("Successfully read config and API keys from CLI and secrets file.");
        Ok(config)
    }        
}

fn uri_to_host_header(uri: &Uri) -> Result<HeaderValue,SamplyBeamError> {
    let hostname: String = uri.host()
        .ok_or(SamplyBeamError::WrongBrokerUri("URI's host is empty."))?.into();
    let port = match uri.port() {
        Some(p) => format!(":{}",p),
        None => String::from(""),
    };
    let host_header = hostname + &port;
    let host_header: HeaderValue = HeaderValue::from_str(&host_header)
        .map_err(|_| SamplyBeamError::WrongBrokerUri("Unable to parse broker URL"))?;
    Ok(host_header)
}
