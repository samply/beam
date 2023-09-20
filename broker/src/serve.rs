use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    extract::{Path, Query},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use serde::Deserialize;
use shared::{
    config, EncryptedMsgTaskRequest, EncryptedMsgTaskResult, HasWaitId, HowLongToBlock, Msg,
    MsgEmpty, MsgId, MsgSigned, EMPTY_VEC_APPORPROXYID,
};
use tokio::{
    sync::{
        broadcast::{Receiver, Sender},
        RwLock,
    },
    time,
};
use tracing::{debug, info, trace, warn};

use crate::{banner, crypto, health::Health, serve_health, serve_pki, serve_tasks, compare_client_server_version};

pub(crate) async fn serve(health: Arc<RwLock<Health>>) -> anyhow::Result<()> {
    let app = serve_tasks::router()
        .merge(serve_pki::router())
        .merge(serve_health::router(health));
    #[cfg(feature = "sockets")]
    let app = app.merge(crate::serve_sockets::router());
    // Middleware needs to be set last
    let app = app
        .layer(axum::middleware::from_fn(shared::middleware::log))
        .layer(axum::middleware::map_response(banner::set_server_header));

    info!(
        "Startup complete. Listening for requests on {}",
        config::CONFIG_CENTRAL.bind_addr
    );
    axum::Server::bind(&config::CONFIG_CENTRAL.bind_addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shared::graceful_shutdown::wait_for_signal())
        .await?;
    Ok(())
}
