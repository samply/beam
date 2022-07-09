#![allow(unused_imports)]

mod serve_axum;
#[cfg(debug_assertions)]
mod devhelper;
mod banner;

use std::{collections::HashMap, sync::Arc};

use shared::*;
use tokio::sync::RwLock;
use tracing::info;

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {    
    #[cfg(debug_assertions)]
    if devhelper::print_example_objects() == true { return Ok(()); }
    
    shared::logger::init_logger()?;
    banner::print_banner();

    let _ = config::CONFIG_CENTRAL.bind_addr; // Initialize config

    let tasks: HashMap<MsgId, MsgSigned<MsgTaskRequest>> = HashMap::new();
    let (new_tasks_tx, _) = tokio::sync::broadcast::channel::<MsgSigned<MsgTaskRequest>>(512);

    let tasks = Arc::new(RwLock::new(tasks));
    let new_tasks_tx = Arc::new(new_tasks_tx);

    serve_axum::serve_axum(tasks, new_tasks_tx).await?;

    Ok(())
}
