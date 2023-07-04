use clap::Parser;
use openssl::x509::X509;
use regex::Regex;

use std::{
    collections::HashMap,
    fs::read_to_string,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::exit,
    str::FromStr,
};

use axum::http::HeaderValue;
use hyper::Uri;
use serde::Deserialize;
use tracing::{debug, info, warn};

use beam_lib::{AppId, ProxyId};
use crate::{
    errors::SamplyBeamError,
};

#[derive(Clone, Debug)]
pub struct Config {
    pub broker_uri: Uri,
    pub broker_host_header: HeaderValue,
    pub bind_addr: SocketAddr,
    pub proxy_id: ProxyId,
    pub api_keys: HashMap<AppId, ApiKey>,
    pub tls_ca_certificates: Vec<X509>,
}

pub type ApiKey = String;

#[derive(Parser, Debug)]
#[clap(
    name("🌈 Samply.Beam.Proxy"),
    version,
    arg_required_else_help(true),
    after_help(crate::config_shared::CLAP_FOOTER)
)]
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

    /// samply.pki: Path to own secret key
    #[clap(long, env, value_parser, default_value = "/run/secrets/privkey.pem")]
    pub privkey_file: PathBuf,

    /// samply.pki: Path to CA Root certificate
    #[clap(long, env, value_parser, default_value = "/run/secrets/root.crt.pem")]
    rootcert_file: PathBuf,

    /// (included for technical reasons)
    #[clap(long, hide(true))]
    test_threads: Option<String>,
}

pub const APP_PREFIX: &str = "APP";

/// Parses API-Keys from the environment like:
/// APP_app1_KEY=App1Secret
/// APP_app2_KEY=App2Secret
fn parse_apikeys(proxy_id: &ProxyId) -> Result<HashMap<AppId, ApiKey>, SamplyBeamError> {
    let env_vars = std::env::vars().collect::<HashMap<String, ApiKey>>();
    let mut api_keys = HashMap::new();
    let pattern = Regex::new(&format!("{APP_PREFIX}_([A-Za-z0-9-]+)_KEY")).expect("This is a valid regex");
    for (env_var_name, secret) in env_vars {
        if let Some(app_name) = pattern.captures_iter(&env_var_name).next().and_then(|cap| cap.get(1)) {
            let Ok(app_id) = AppId::new(&format!("{}.{proxy_id}", app_name.as_str())) else {
                // Only warn here as there might be other env vars that could match this pattern
                warn!("Failed to create app id from env var: {env_var_name}. Skipping");
                continue;
            };
            if secret.is_empty() {
                return Err(SamplyBeamError::ConfigurationFailed(format!(
                    "Please supply a non empty API key for client {app_id}",
                )));
            }
            api_keys.insert(app_id, secret);
        }
    }
    Ok(api_keys)
}

impl crate::config::Config for Config {
    fn load() -> Result<Config, SamplyBeamError> {
        let cli_args = CliArgs::parse();
        beam_lib::set_broker_id(cli_args.broker_url.host().unwrap().to_string());
        let proxy_id = ProxyId::new(&cli_args.proxy_id).map_err(|e| {
            SamplyBeamError::ConfigurationFailed(format!(
                "Invalid Beam ID \"{}\" supplied: {}",
                cli_args.proxy_id, e
            ))
        })?;
        let api_keys = parse_apikeys(&proxy_id)?;
        if api_keys.is_empty() {
            return Err(SamplyBeamError::ConfigurationFailed(format!("No API keys have been defined. Please set environment vars à la {0}_<clientname>_KEY=<key>", APP_PREFIX)));
        }
        let tls_ca_certificates = crate::crypto::load_certificates_from_dir(
            cli_args.tls_ca_certificates_dir,
        )
        .map_err(|e| {
            SamplyBeamError::ConfigurationFailed(format!(
                "Unable to read from TLS CA directory: {}",
                e
            ))
        })?;
        let config = Config {
            broker_host_header: uri_to_host_header(&cli_args.broker_url)?,
            broker_uri: cli_args.broker_url,
            bind_addr: cli_args.bind_addr,
            proxy_id,
            api_keys,
            tls_ca_certificates,
        };
        info!("Successfully read config and API keys from CLI and secrets file.");
        Ok(config)
    }
}

fn uri_to_host_header(uri: &Uri) -> Result<HeaderValue, SamplyBeamError> {
    let hostname: String = uri
        .host()
        .ok_or(SamplyBeamError::WrongBrokerUri("URI's host is empty."))?
        .into();
    let port = match uri.port() {
        Some(p) => format!(":{}", p),
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
            ("app1-test", "App1Secret"),
            ("app2", "App2Secret"),
            ("app3", "App3Secret"),
        ];
        for (i, (app, key)) in apps.iter().enumerate() {
            std::env::set_var(format!("APP_{i}_ID"), app);
            std::env::set_var(format!("APP_{i}_KEY"), key);
            std::env::set_var(format!("APP_{app}_KEY"), key);
        }
        const BROKER_ID: &str = "broker.samply.de";
        beam_lib::set_broker_id(BROKER_ID.to_string());
        let parsed = parse_apikeys(&ProxyId::new(&format!("proxy.{BROKER_ID}")).unwrap()).unwrap();
        assert_eq!(parsed.len(), apps.len() * 2);
    }
}
