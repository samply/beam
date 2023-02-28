#![allow(unused_imports)]

mod serve;
mod serve_tasks;
mod serve_health;
mod serve_pki;
mod banner;
mod expire;
mod crypto;
mod health;

use std::{collections::HashMap, sync::Arc, time::Duration};

use backoff::{future::{retry, retry_notify}, ExponentialBackoff, ExponentialBackoffBuilder};
use shared::{*, config::CONFIG_CENTRAL};
use tokio::sync::RwLock;
use tracing::{info, warn};

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {    
    shared::config::prepare_env();
    shared::logger::init_logger()?;
    banner::print_banner();

    let (senders, health) = health::Health::make();
    let cert_getter = crypto::build_cert_getter(senders.vault)?;

    shared::crypto::init_cert_getter(cert_getter);
    shared::crypto::init_ca_chain().await;
    #[cfg(debug_assertions)]
    if shared::examples::print_example_objects() { return Ok(()); }
    
    let _ = config::CONFIG_CENTRAL.bind_addr; // Initialize config

    serve::serve(health).await?;

    Ok(())
}
