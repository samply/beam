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

    check_clientid().await?;

    serve::serve().await?;
    Ok(())
}

async fn check_clientid() -> Result<(),SamplyBeamError> {
    let client_id_cli = &config::CONFIG_PROXY.proxy_id;
    let _public_info = shared::crypto::get_cert_and_client_by_cname_as_pemstr(client_id_cli).await
        .ok_or_else(|| SamplyBeamError::VaultError(format!("Unable to fetch your certificate from vault. Is your local Client ID really {}?", client_id_cli)))?;

    // TODO: Check if certificate CNAME matches client_id_cli

    Ok(())
}