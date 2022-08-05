use clap::Parser;
use openssl::x509::X509;

use std::{net::SocketAddr, process::exit, collections::HashMap, fs::read_to_string, str::FromStr, path::{Path, PathBuf}};

use axum::http::HeaderValue;
use hyper::Uri;
use serde::Deserialize;
use tracing::{info, debug};

use crate::{errors::SamplyBeamError, beam_id::{BeamId, ProxyId, AppId, self, BrokerId}};

#[derive(Clone,Debug)]
pub struct Config {
    pub broker_uri: Uri,
    pub broker_host_header: HeaderValue,
    pub bind_addr: SocketAddr,
    pub pki_address: Uri,
    pub pki_realm: String,
    pub proxy_id: ProxyId,
    pub api_keys: HashMap<AppId,ApiKey>,
    pub tls_ca_certificates: Vec<X509>
}

pub type ApiKey = String;

#[derive(Parser,Debug)]
#[clap(name("🌈 Samply.Beam.Proxy"), version, arg_required_else_help(true), after_help(crate::config_shared::CLAP_FOOTER))]
pub struct CliArgs {
    /// Local bind address
    #[clap(long, env, value_parser, default_value_t = SocketAddr::from_str("0.0.0.0:8081").unwrap())]
    pub bind_addr: SocketAddr,

    /// Outgoing HTTP proxy: Directory with CA certificates to trust for TLS connections (e.g. /etc/samply/cacerts/)
    #[clap(long, env, value_parser)]
    pub tls_ca_certificates_dir: Option<PathBuf>,
    
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

pub const APP_PREFIX: &str = "APP";

/// Parses API-Keys from the environment, expecting:
/// APP_0_ID=app1
/// APP_0_KEY=App1Secret
/// APP_1_ID=app2
/// APP_1_KEY=App2Secret
fn parse_apikeys(proxy_id: &ProxyId) -> Result<HashMap<AppId,ApiKey>,SamplyBeamError>{
    let vars = std::env::vars().collect::<HashMap<String,ApiKey>>();
    let mut api_keys = HashMap::new();
    let mut i = 0;
    while let Some(app_id) = vars.get(&format!("{APP_PREFIX}_{i}_ID")) {
        if let Some(api_key) = vars.get(&format!("{APP_PREFIX}_{i}_KEY")) {
            let app_id = AppId::new(&format!("{app_id}.{proxy_id}"))?;
            if api_key.is_empty() {
                return Err(SamplyBeamError::ConfigurationFailed(format!("Unable to assign empty API key for client {}", app_id)));
            }
            api_keys.insert(app_id, api_key.clone());
        }
        i += 1;
    }
    Ok(api_keys)
}

impl crate::config::Config for Config {
    fn load() -> Result<Config,SamplyBeamError> {
        let cli_args = CliArgs::parse();
        BrokerId::set_broker_id(&cli_args.broker_url.host().unwrap().to_string());
        let proxy_id = ProxyId::new(&cli_args.proxy_id)
            .map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Invalid Beam ID \"{}\" supplied: {}", cli_args.proxy_id, e)))?;
        let api_keys = parse_apikeys(&proxy_id)?;
        if api_keys.is_empty() {
            return Err(SamplyBeamError::ConfigurationFailed(format!("No API keys have been defined. Please set environment vars à la {0}_0_ID=<clientname>, {0}_0_KEY=<key>", APP_PREFIX)));
        }
        let tls_ca_certificates = crate::crypto::load_certificates_from_dir(cli_args.tls_ca_certificates_dir)
            .map_err(|e| SamplyBeamError::ConfigurationFailed(format!("Unable to read from TLS CA directory: {}", e)))?;
        let config = Config {
            broker_host_header: uri_to_host_header(&cli_args.broker_url)?,
            broker_uri: cli_args.broker_url,
            pki_address: cli_args.pki_address,
            bind_addr: cli_args.bind_addr,
            pki_realm: cli_args.pki_realm,
            proxy_id,
            api_keys,
            tls_ca_certificates
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

#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    #[test]
    fn test_parse_apikeys() {
        let apps = [
            ("app1", "App1Secret"),
            ("app2", "App2Secret"),
            ("app3", "App3Secret"),
        ];
        for (i, (app, key)) in apps.iter().enumerate() {
            std::env::set_var(format!("APP_{i}_ID"), app);
            std::env::set_var(format!("APP_{i}_KEY"), key);
        }
        const BROKER_ID: &str = "broker.samply.de";
        BrokerId::set_broker_id(&BROKER_ID.to_string());
        let parsed = parse_apikeys(&ProxyId::new(&format!("proxy.{BROKER_ID}")).unwrap()).unwrap();
        assert_eq!(parsed.len(), apps.len());
    }
}