use std::{fs::read_to_string, net::SocketAddr, path::PathBuf};

use crate::{
    errors::SamplyBeamError,
};
use axum::http::Uri;
use clap::Parser;
use static_init::dynamic;
use std::str::FromStr;
use tracing::info;

#[derive(Parser, Debug)]
#[clap(
    name("🌈 Samply.Beam.Broker"),
    version,
    arg_required_else_help(true),
    after_help(crate::config_shared::CLAP_FOOTER)
)]
struct CliArgs {
    /// Local bind address
    #[clap(long, env, value_parser, default_value_t = SocketAddr::from_str("0.0.0.0:8080").unwrap())]
    bind_addr: SocketAddr,

    /// Outgoing HTTP proxy: Directory with CA certificates to trust for TLS connections (e.g. /etc/samply/cacerts/)
    #[clap(long, env, value_parser)]
    pub tls_ca_certificates_dir: Option<PathBuf>,

    /// The broker's base URL, e.g. https://beam.samply.de
    #[clap(long, env, value_parser)]
    broker_url: Uri,

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

    /// samply.pki: Path to CA Root certificate
    #[clap(long, env, value_parser, default_value = "/run/secrets/root.crt.pem")]
    rootcert_file: PathBuf,

    /// The API key for accessing monitoring endpoints of the broker
    #[clap(long, env, value_parser)]
    monitoring_api_key: Option<String>,

    /// (included for technical reasons)
    #[clap(long, hide(true))]
    test_threads: Option<String>,
}

pub struct Config {
    pub bind_addr: SocketAddr,
    pub pki_address: Uri,
    pub pki_realm: String,
    pub pki_token: String,
    pub tls_ca_certificates_dir: Option<PathBuf>,
    pub monitoring_api_key: Option<String>,
}

impl crate::config::Config for Config {
    fn load() -> Result<Self, SamplyBeamError> {
        let cli_args = CliArgs::parse();
        beam_lib::set_broker_id(cli_args.broker_url.host().unwrap().to_string());
        let pki_token = read_to_string(&cli_args.pki_apikey_file)
            .map_err(|e| {
                SamplyBeamError::ConfigurationFailed(format!(
                    "Unable to read PKI API key at {}: {}",
                    &cli_args.pki_apikey_file.to_string_lossy(),
                    e
                ))
            })?
            .trim()
            .to_string();

        info!("Successfully read config and API keys from CLI and secrets files.");
        let config = Config {
            bind_addr: cli_args.bind_addr,
            pki_address: cli_args.pki_address,
            pki_realm: cli_args.pki_realm,
            pki_token,
            tls_ca_certificates_dir: cli_args.tls_ca_certificates_dir,
            monitoring_api_key: cli_args.monitoring_api_key,
        };
        Ok(config)
    }
}
