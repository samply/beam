use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    extract::{DefaultBodyLimit, FromRef, Path, Query},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Extension, Json, Router,
};
use serde::Deserialize;
use shared::{
    EncryptedMsgTaskRequest, EncryptedMsgTaskResult, HasWaitId, HowLongToBlock, Msg,
    MsgEmpty, MsgId, MsgSigned, EMPTY_VEC_APPORPROXYID,
};
use tokio::{
    net::TcpListener, sync::{
        broadcast::{Receiver, Sender},
        RwLock,
    }, time
};
use tracing::{debug, info, trace, warn};

use crate::{banner, compare_client_server_version, config::Config, crypto, serve_health::{self, Health}, serve_pki, serve_tasks};

pub(crate) async fn serve(broker_state: BrokerState) -> anyhow::Result<()> {
    let bind_addr = broker_state.config.bind_addr;
    let app = serve_tasks::router()
        .merge(serve_pki::router())
        .merge(serve_health::router(broker_state));
    #[cfg(feature = "sockets")]
    let app = app.merge(crate::serve_sockets::router());
    // Middleware needs to be set last
    let app = app
        .layer(axum::middleware::from_fn(shared::middleware::log))
        .layer(axum::middleware::map_response(banner::set_server_header))
        .layer(DefaultBodyLimit::disable());

    info!("Startup complete. Listening for requests on {bind_addr}");
    axum::serve(TcpListener::bind(bind_addr).await?, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shared::graceful_shutdown::wait_for_signal())
        .await?;
    Ok(())
}

#[derive(Debug, FromRef, Clone)]
pub struct BrokerState {
    pub health: Arc<RwLock<Health>>,
    pub config: &'static Config,
}

impl BrokerState {
    pub fn new(health: Arc<RwLock<Health>>, config: Config) -> Self {
        Self {
            health,
            config: Box::leak(Box::new(config)),
        }
    }
}