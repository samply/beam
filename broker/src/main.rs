#![allow(unused_imports)]

mod banner;
mod crypto;
mod health;
mod serve;
mod serve_health;
mod serve_pki;
mod serve_tasks;
#[cfg(feature = "sockets")]
mod serve_sockets;
mod task_manager;
mod compare_client_server_version;

use std::{collections::HashMap, sync::Arc, time::Duration};

use crypto::GetCertsFromPki;
use health::{Health, InitStatus};
use once_cell::sync::Lazy;
use shared::{config::CONFIG_CENTRAL, *, errors::SamplyBeamError};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    shared::logger::init_logger()?;
    banner::print_banner();

    let health = Arc::new(RwLock::new(Health::default()));
    let cert_getter = GetCertsFromPki::new(health.clone())?;

    shared::crypto::init_cert_getter(cert_getter);
    tokio::task::spawn(init_broker_ca_chain(health.clone()));
    #[cfg(debug_assertions)]
    if shared::examples::print_example_objects() {
        return Ok(());
    }

    Lazy::force(&config::CONFIG_CENTRAL); // Initialize config

    serve::serve(health).await?;

    Ok(())
}

async fn init_broker_ca_chain(health: Arc<RwLock<Health>>) {
    {
        health.write().await.initstatus = health::InitStatus::FetchingIntermediateCert
    }
    shared::crypto::init_ca_chain().await.expect("Failed to init broker ca chain");
    health.write().await.initstatus = health::InitStatus::Done;
}
