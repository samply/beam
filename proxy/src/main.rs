#![allow(unused_imports)]

use hyper::{Client, client::HttpConnector};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use shared::{config, config_proxy::Config};
use shared::errors::SamplyBeamError;
use tracing::{warn, info, debug, error};

mod auth;
mod serve;
mod serve_health;
mod serve_tasks;
mod banner;
mod crypto;

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    shared::config::prepare_env();
    shared::logger::init_logger()?;
    banner::print_banner();

    let config = config::CONFIG_PROXY.clone();
    let client = shared::http_proxy::build_hyper_client(&config.tls_ca_certificates)
        .map_err(SamplyBeamError::HttpProxyProblem)?;

    let ec =init_crypto(config.clone(), client.clone()).await;
    if let Err(e) = ec {
        error!("{}",e);
        std::process::exit(1);
    }

    serve::serve(config, client).await?;
    Ok(())
}

async fn init_crypto(config: Config, client: Client<ProxyConnector<HttpsConnector<HttpConnector>>>) -> Result<(),SamplyBeamError> {
    shared::crypto::init_cert_getter(crypto::build_cert_getter(config.clone(), client.clone())?);
    
    let public_info = shared::crypto::get_all_certs_and_clients_by_cname_as_pemstr(&config.proxy_id).await
        .ok_or_else(|| SamplyBeamError::VaultError(format!("Unable to fetch your certificate from vault. Is your Proxy ID really {}?", config.proxy_id)))?;

    debug!("Got {} certificates from the central CA", public_info.len());

    let (serial, cname) = shared::config_shared::init_crypto_for_proxy().await?;
    if cname != config.proxy_id.to_string() {
        return Err(SamplyBeamError::ConfigurationFailed(format!("Unable to retrieve a certificate matching your Proxy ID. Expected {}, got {}. Please check your configuration", cname, config.proxy_id.to_string())));
    }

    info!("Certificate retrieved for our proxy ID {cname} (serial {serial})");

    Ok(())
}
