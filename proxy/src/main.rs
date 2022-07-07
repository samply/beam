#![allow(unused_imports)]

use config::{CONFIG, Config};
use shared::errors::SamplyBrokerError;
use tracing::warn;

mod auth;
mod reverse;
mod config;

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    shared::logger::init_logger()?;

    check_clientid().await?;

    reverse::reverse_proxy().await?;
    Ok(())
}

async fn check_clientid() -> Result<(),SamplyBrokerError> {
    let client_id_cli = &CONFIG.client_id;
    let _public_info = shared::crypto::get_cert_and_client_by_cname_as_pemstr(client_id_cli).await
        .ok_or(SamplyBrokerError::VaultError(format!("Unable to fetch your certificate from vault. Is your local Client ID really {}?", client_id_cli)))?;

    // TODO: Check if certificate CNAME matches client_id_cli

    Ok(())
}