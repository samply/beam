use std::{collections::HashMap, sync::Arc};

use axum::{
    http::{StatusCode, header},
    routing::{get, post},
    Extension, Json, Router, extract::{Query, Path}, response::IntoResponse
};
use serde::{Deserialize};
use shared::{EncryptedMsgTaskRequest, EncryptedMsgTaskResult, MsgId, HowLongToBlock, HasWaitId, MsgSigned, MsgEmpty, Msg, EMPTY_VEC_APPORPROXYID, config};
use tokio::{sync::{broadcast::{Sender, Receiver}, RwLock}, time};
use tracing::{debug, info, trace, warn};

use crate::{serve_tasks, serve_health, serve_pki, crypto::GetCertsFromPki, banner};

pub(crate) async fn serve() -> anyhow::Result<()> {
    let app = 
        serve_tasks::router()
        // .merge(serve_pki::router())
        .merge(serve_pki::router())
        .merge(serve_health::router())
        .layer(axum::middleware::map_response(banner::set_server_header));

    // Graceful shutdown handling
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::mpsc::channel(1);

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await
                .expect("Unable to listen for Ctrl+C for graceful shutdown");
        if shutdown_tx.blocking_send(()).is_err() {
            warn!("Unable to send signal for clean shutdown... ignoring.");
        }
    });

    info!("Listening for requests on {}", config::CONFIG_CENTRAL.bind_addr);
    axum::Server::bind(&config::CONFIG_CENTRAL.bind_addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(async {
            shutdown_rx.recv().await;
            info!("Shutting down.");
        })
        .await?;
    Ok(())
}
