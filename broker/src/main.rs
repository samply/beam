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

use health::{Senders, InitStatus};
use shared::{config::CONFIG_CENTRAL, *, errors::SamplyBeamError};
use tokio::sync::{RwLock, watch};
use tracing::{error, info, warn};

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    shared::config::prepare_env();
    shared::logger::init_logger()?;
    banner::print_banner();

    let (Senders { init: init_status_sender, vault: vault_status_sender}, health) = health::Health::make();
    let cert_getter = crypto::build_cert_getter(vault_status_sender)?;

    shared::crypto::init_cert_getter(cert_getter);
    tokio::task::spawn(init_broker_ca_chain(init_status_sender));
    #[cfg(debug_assertions)]
    if shared::examples::print_example_objects() {
        return Ok(());
    }

    let _ = config::CONFIG_CENTRAL.bind_addr; // Initialize config

    serve::serve(health).await?;

    Ok(())
}

async fn init_broker_ca_chain(sender: watch::Sender<InitStatus>) {
    sender.send_replace(health::InitStatus::FetchingIntermediateCert);
    shared::crypto::init_ca_chain().await.expect("Failed to init broker ca chain");
    sender.send_replace(health::InitStatus::Done);
}
