#![allow(unused_imports)]

mod banner;
mod crypto;
mod config;
mod serve;
mod serve_health;
mod serve_pki;
mod serve_tasks;
#[cfg(feature = "sockets")]
mod serve_sockets;
mod task_manager;
mod compare_client_server_version;

use std::{collections::HashMap, sync::Arc, time::Duration};

use clap::Parser;
use crypto::GetCertsFromPki;
use serve_health::{Health, InitStatus};
use once_cell::sync::Lazy;
use shared::{errors::SamplyBeamError, openssl::x509::X509, *};
use tokio::sync::RwLock;
use tracing::{error, info, warn};

use crate::{config::CliArgs, serve::BrokerState};

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    let args = CliArgs::parse();
    let _log_guard = shared::logger::init_logger(&args.log_options)?;
    banner::print_banner();
    let config = config::Config::load(args)?;

    let health = Arc::new(RwLock::new(Health::default()));
    let cert_getter = GetCertsFromPki::new(health.clone(), &config)?;

    shared::crypto::init_cert_getter(cert_getter);
    tokio::task::spawn(init_broker_ca_chain(health.clone(), config.rootcert.clone()));

    serve::serve(BrokerState::new(health, config)).await?;

    Ok(())
}

async fn init_broker_ca_chain(health: Arc<RwLock<Health>>, rootcert: X509) {
    {
        health.write().await.initstatus = InitStatus::FetchingIntermediateCert
    }
    shared::crypto::init_ca_chain(&rootcert).await.expect("Failed to init broker ca chain");
    health.write().await.initstatus = InitStatus::Done;
}
