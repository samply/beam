use std::{collections::HashMap, sync::Arc};

use axum::{
    http::{StatusCode, header},
    routing::{get, post},
    Extension, Json, Router, extract::{Query, Path}, response::IntoResponse
};
use serde::{Deserialize};
use shared::{MsgTaskRequest, MsgTaskResult, MsgId, HowLongToBlock, HasWaitId, MsgSigned, MsgEmpty, Msg, EMPTY_VEC_APPORPROXYID, config};
use tokio::{sync::{broadcast::{Sender, Receiver}, RwLock}, time};
use tracing::{debug, info, trace};

use crate::{serve_tasks, serve_health};

pub(crate) async fn serve() -> anyhow::Result<()> {
    let app = 
        serve_tasks::router()
        // .merge(serve_pki::router())
        .merge(serve_health::router());

    // Graceful shutdown handling
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);

    ctrlc::set_handler(move || tx.blocking_send(()).expect("Could not send shutdown signal on channel."))
        .expect("Error setting handler for graceful shutdown.");

    info!("Listening for requests on {}", config::CONFIG_CENTRAL.bind_addr);
    axum::Server::bind(&config::CONFIG_CENTRAL.bind_addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(async {
            rx.recv().await;
            info!("Shutting down.");
        })
        .await?;
    Ok(())
}
