use clap::Parser;
use openssl::x509::X509;

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
use tracing::{debug, info};

use crate::{
    beam_id::{self, AppId, BeamId, BrokerId, ProxyId, AppOrProxyId},
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
    pub permission_manager: PermissionManager,
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
    
    /// A whitelist of apps or proxies that messages may be sent to, e.g. ["app1.proxy1.broker", "proxy2.broker", ...]
    #[clap(long, env, value_parser)]
    pub allowed_receivers: Option<String>,

    /// A blacklist of apps or proxies that messages may not be sent to, e.g. ["app1.proxy1.broker", "proxy2.broker", ...]
    #[clap(long, env, value_parser)]
    pub blocked_receivers: Option<String>,

    /// A whitelist of apps or proxies that may send messages to this proxy, e.g. ["app1.proxy1.broker", "proxy2.broker", ...]
    #[clap(long, env, value_parser)]
    pub allowed_remotes: Option<String>,

    /// A blacklist of apps or proxies that may not send messages to this proxy, e.g. ["app1.proxy1.broker", "proxy2.broker", ...]
    #[clap(long, env, value_parser)]
    pub blocked_remotes: Option<String>,

    /// (included for technical reasons)
    #[clap(long, hide(true))]
    test_threads: Option<String>,
}

pub const APP_PREFIX: &str = "APP";

/// Parses API-Keys from the environment, expecting:
/// APP_0_ID=app1
/// APP_0_KEY=App1Secret
/// APP_1_ID=app2
/// APP_1_KEY=App2Secret
fn parse_apikeys(proxy_id: &ProxyId) -> Result<HashMap<AppId, ApiKey>, SamplyBeamError> {
    let vars = std::env::vars().collect::<HashMap<String, ApiKey>>();
    let mut api_keys = HashMap::new();
    let mut i = 0;
    while let Some(app_id) = vars.get(&format!("{APP_PREFIX}_{i}_ID")) {
        if let Some(api_key) = vars.get(&format!("{APP_PREFIX}_{i}_KEY")) {
            let app_id = AppId::new(&format!("{app_id}.{proxy_id}"))?;
            if api_key.is_empty() {
                return Err(SamplyBeamError::ConfigurationFailed(format!(
                    "Unable to assign empty API key for client {}",
                    app_id
                )));
            }
            api_keys.insert(app_id, api_key.clone());
        }
        i += 1;
    }
    Ok(api_keys)
}

#[derive(Debug, Clone)]
pub struct PermissionManager {
    pub recv_allow_list: Option<Vec<AppOrProxyId>>,
    pub recv_block_list: Option<Vec<AppOrProxyId>>,
    pub send_allow_list: Option<Vec<AppOrProxyId>>,
    pub send_block_list: Option<Vec<AppOrProxyId>>,
}

impl PermissionManager {
    pub fn allowed_to_recieve(&self, from: &AppOrProxyId) -> bool {
        Self::check_permissions_with(&self.recv_allow_list,&self.recv_block_list, from)
    }

    pub fn allowed_to_send(&self, to: &AppOrProxyId) -> bool {
        Self::check_permissions_with(&self.send_allow_list, &self.send_block_list, to)
    }

    fn check_permissions_with(allow: &Option<Vec<AppOrProxyId>>, deny: &Option<Vec<AppOrProxyId>>, beam_id: &AppOrProxyId) -> bool {
        match (allow, deny) {
            (None, None) => true,
            (None, Some(block_list)) => !Self::contains(&block_list, beam_id),
            (Some(allow_list), None) => Self::contains(&allow_list, beam_id),
            (Some(allow_list), Some(block_list)) => !Self::contains(&block_list, beam_id) || Self::contains(&allow_list, beam_id)
        }
    }

    fn contains(ids: &Vec<AppOrProxyId>, needle: &AppOrProxyId) -> bool {
        ids.iter().find(|id| match id {
            AppOrProxyId::AppId(app) => needle == app,
            AppOrProxyId::ProxyId(proxy) => proxy == &needle.get_proxy_id(),
        }).is_some()
    }
}

impl crate::config::Config for Config {
    fn load() -> Result<Config, SamplyBeamError> {
        let cli_args = CliArgs::parse();
        BrokerId::set_broker_id(cli_args.broker_url.host().unwrap().to_string());
        let proxy_id = ProxyId::new(&cli_args.proxy_id).map_err(|e| {
            SamplyBeamError::ConfigurationFailed(format!(
                "Invalid Beam ID \"{}\" supplied: {}",
                cli_args.proxy_id, e
            ))
        })?;
        let api_keys = parse_apikeys(&proxy_id)?;
        if api_keys.is_empty() {
            return Err(SamplyBeamError::ConfigurationFailed(format!("No API keys have been defined. Please set environment vars à la {0}_0_ID=<clientname>, {0}_0_KEY=<key>", APP_PREFIX)));
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
            permission_manager: PermissionManager {
                recv_allow_list: parse_to_list_of_ids(cli_args.allowed_remotes)?,
                recv_block_list: parse_to_list_of_ids(cli_args.blocked_remotes)?,
                send_allow_list: parse_to_list_of_ids(cli_args.allowed_receivers)?,
                send_block_list: parse_to_list_of_ids(cli_args.blocked_receivers)?
            }
        };
        info!("Successfully read config and API keys from CLI and secrets file.");
        Ok(config)
    }
}

fn parse_to_list_of_ids(input: Option<String>) -> Result<Option<Vec<AppOrProxyId>>, SamplyBeamError> {
    if let Some(app_list_str) = input {
        Ok(Some(serde_json::from_str(&app_list_str).map_err(|e| SamplyBeamError::ConfigurationFailed(
            format!("Failed to parse: {app_list_str} to a list of beam ids: {e}.\n The requiered format is a json array of strings that match the beam id spec see the system architecture section in the readme.")
        ))?))
    } else {
        Ok(None)
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
            ("app1", "App1Secret"),
            ("app2", "App2Secret"),
            ("app3", "App3Secret"),
        ];
        for (i, (app, key)) in apps.iter().enumerate() {
            std::env::set_var(format!("APP_{i}_ID"), app);
            std::env::set_var(format!("APP_{i}_KEY"), key);
        }
        const BROKER_ID: &str = "broker.samply.de";
        BrokerId::set_broker_id(BROKER_ID.to_string());
        let parsed = parse_apikeys(&ProxyId::new(&format!("proxy.{BROKER_ID}")).unwrap()).unwrap();
        assert_eq!(parsed.len(), apps.len());
    }

    #[test]
    fn test_parse_app_list() {
        const BROKER_ID: &str = "broker";
        BrokerId::set_broker_id(BROKER_ID.to_string());
        assert_eq!(
            parse_to_list_of_ids(Some(r#"["app1.proxy1.broker", "proxy1.broker"]"#.to_string())).unwrap(),
            Some(vec![AppOrProxyId::AppId(AppId::new("app1.proxy1.broker").unwrap()),AppOrProxyId::ProxyId(ProxyId::new("proxy1.broker").unwrap())])
        );
        assert_eq!(parse_to_list_of_ids(None).unwrap(), None);
    }
}
