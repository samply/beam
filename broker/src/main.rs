#![allow(unused_imports)]

mod serve;
mod serve_tasks;
mod serve_health;
mod banner;

use std::{collections::HashMap, sync::Arc};

use shared::*;
use tokio::sync::RwLock;
use tracing::info;

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {    
    shared::config::prepare_env();

    #[cfg(debug_assertions)]
    if shared::examples::print_example_objects() { return Ok(()); }
    
    shared::logger::init_logger()?;
    banner::print_banner();

    let _ = config::CONFIG_CENTRAL.bind_addr; // Initialize config
    let _ = shared::crypto_jwt::sign_to_jwt("Init config").await?; // Initialize config

    serve::serve().await?;

    Ok(())
}
