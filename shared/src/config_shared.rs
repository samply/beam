use crate::SamplyBrokerError;
use std::path::PathBuf;
use hyper::Uri;
use clap::Parser;

/// Settings for Samply.Broker.Shared
#[derive(Parser,Debug)]
#[clap(author, version, about, long_about = None)]
pub(crate) struct VaultConfig {
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

    // TODO: The following arguments have been added for compatibility reasons with the proxy config. Find another way to merge configs.
    /// (included for technical reasons)
    #[clap(long, env, value_parser)]
    broker_url: Option<Uri>,

    /// (included for technical reasons)
    #[clap(long, env, value_parser)]
    client_id: Option<String>,
}

pub(crate) fn get_config() -> VaultConfig {
    let cli_args = VaultConfig::parse();
    cli_args
}