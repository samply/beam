#![allow(unused_imports)]

mod serve;
mod serve_tasks;
mod serve_health;
mod serve_pki;
mod banner;
mod expire;
mod crypto;

use std::{collections::HashMap, sync::Arc};

use shared::*;
use tokio::sync::RwLock;
use tracing::info;

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {    
    shared::config::prepare_env();

    let cert_getter = crypto::build_cert_getter()?;
    shared::crypto::init_cert_getter(cert_getter);
    shared::crypto::CERT_CACHE.write().await.set_root_cert(&config::CONFIG_SHARED.root_cert);
    #[cfg(debug_assertions)]
    if shared::examples::print_example_objects() { return Ok(()); }
    
    shared::logger::init_logger()?;
    banner::print_banner();

    let _ = config::CONFIG_CENTRAL.bind_addr; // Initialize config

    serve::serve().await?;

    Ok(())
}
