use std::{net::SocketAddr, path::PathBuf, fs::read_to_string};

use axum::http::Uri;
use clap::Parser;
use crate::{errors::SamplyBrokerError, ClientId};
use tracing::info;
use std::str::FromStr;

/// Settings for Samply.Broker.Central
#[derive(Parser,Debug)]
#[clap(author, version, about, long_about = None, arg_required_else_help(true))]
pub struct CliArgs {
    /// local bind address
    #[clap(long, env, value_parser, default_value_t = SocketAddr::from_str("0.0.0.0:8080").unwrap())]
    pub bind_addr: SocketAddr,

    /// samply.pki: URL to HTTPS endpoint
    #[clap(long, env, value_parser)]
    pub pki_address: Uri,

    /// samply.pki: Authentication realm
    #[clap(long, env, value_parser, default_value = "samply_pki")]
    pub pki_realm: String,

    /// samply.pki: File containing the authentication token
    #[clap(long, env, value_parser, default_value = "/run/secrets/pki.secret")]
    pub pki_apikey_file: PathBuf,
}

pub struct Config {
    pub bind_addr: SocketAddr,
    pub pki_address: Uri,
    pub pki_realm: String,
    pub pki_token: String,
}

pub(crate) fn get_config() -> Result<Config,SamplyBrokerError> {
    let cli_args = CliArgs::parse();
    let pki_token = read_to_string(&cli_args.pki_apikey_file)
        .map_err(|e| SamplyBrokerError::ConfigurationFailed(format!("Unable to read PKI API key at {}: {}", &cli_args.pki_apikey_file.to_string_lossy(), e)))?.trim().to_string();

    info!("Successfully read config and API keys from CLI and secrets files.");
    let config = Config {
        bind_addr: cli_args.bind_addr,
        pki_address: cli_args.pki_address,
        pki_realm: cli_args.pki_realm,
        pki_token,
    };
    Ok(config)
}
