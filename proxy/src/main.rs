#![allow(unused_imports)]

use shared::config;
use shared::errors::SamplyBeamError;
use tracing::{warn, info, debug};

mod auth;
mod serve;
mod serve_health;
mod serve_tasks;
mod banner;

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    shared::config::prepare_env();
    shared::logger::init_logger()?;
    banner::print_banner();

    let config = config::CONFIG_PROXY.clone();
    check_clientid().await?;

    serve::serve(config).await?;
    Ok(())
}

async fn check_clientid() -> Result<(),SamplyBeamError> {
    let config = config::CONFIG_PROXY.clone();
    let _public_info = shared::crypto::get_cert_and_client_by_cname_as_pemstr(&config.proxy_id).await
        .ok_or_else(|| SamplyBeamError::VaultError(format!("Unable to fetch your certificate from vault. Is your Proxy ID really {}?", config.proxy_id)))?;

    let (serial, cname) = shared::config_shared::init_crypto().await?;
    if cname != config.proxy_id.to_string() {
        return Err(SamplyBeamError::ConfigurationFailed(format!("Unable to retrieve a certificate matching your Proxy ID. Expected {}, got {}. Please check your configuration", cname, config.proxy_id.to_string())));
    }

    info!("Certificate retrieved for our proxy ID {cname} (serial {serial})");

    Ok(())
}