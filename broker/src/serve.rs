use std::{collections::HashMap, sync::Arc, net::SocketAddr};

use axum::{
    http::{StatusCode, header},
    routing::{get, post},
    Extension, Json, Router, extract::{Query, Path}, response::IntoResponse
};
use serde::{Deserialize};
use shared::{EncryptedMsgTaskRequest, EncryptedMsgTaskResult, MsgId, HowLongToBlock, HasWaitId, MsgSigned, MsgEmpty, Msg, EMPTY_VEC_APPORPROXYID, config};
use tokio::{sync::{broadcast::{Sender, Receiver}, RwLock}, time};
use tracing::{debug, info, trace, warn};

use crate::{serve_tasks, serve_health, serve_pki, banner, health::Health, crypto};

pub(crate) async fn serve(health: Arc<RwLock<Health>>) -> anyhow::Result<()> {
    let app = 
        serve_tasks::router()
        .merge(serve_pki::router())
        .merge(serve_health::router(health))
        .layer(axum::middleware::from_fn(shared::middleware::log))
        .layer(axum::middleware::map_response(banner::set_server_header));

    info!("Startup complete. Listening for requests on {}", config::CONFIG_CENTRAL.bind_addr);
    axum::Server::bind(&config::CONFIG_CENTRAL.bind_addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shared::graceful_shutdown::wait_for_signal())
        .await?;
    Ok(())
}
