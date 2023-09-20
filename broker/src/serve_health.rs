use std::{sync::Arc, time::{Duration, SystemTime}};

use axum::{extract::{State, Path}, http::StatusCode, routing::get, Json, Router, TypedHeader, headers::{Authorization, authorization::Basic}, response::Response};
use beam_lib::ProxyId;
use serde::{Serialize, Deserialize};
use shared::{crypto_jwt::Authorized, Msg, config::CONFIG_CENTRAL};
use tokio::sync::RwLock;

use crate::{health::{Health, VaultStatus, Verdict, ProxyStatus, InitStatus}, compare_client_server_version::log_version_mismatch};

#[derive(Serialize)]
struct HealthOutput {
    summary: Verdict,
    vault: VaultStatus,
    init_status: InitStatus
}

pub(crate) fn router(health: Arc<RwLock<Health>>) -> Router {
    Router::new()
        .route("/v1/health", get(handler))
        .route("/v1/health/proxies/:proxy_id", get(proxy_health))
        .route("/v1/health/proxies", get(get_all_proxies))
        .route("/v1/control", get(get_control_tasks).layer(axum::middleware::from_fn(log_version_mismatch)))
        .with_state(health)
}

// GET /v1/health
async fn handler(
    State(state): State<Arc<RwLock<Health>>>,
) -> (StatusCode, Json<HealthOutput>) {
    let state = state.read().await;
    let (statuscode, summary) = match (state.initstatus, state.vault) {
        (InitStatus::Done, VaultStatus::Ok) => (StatusCode::OK, Verdict::Healthy),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Verdict::Unhealthy,
        ),
    };
    let health_as_json = HealthOutput {
        summary,
        vault: state.vault,
        init_status: state.initstatus
    };
    (statuscode, Json(health_as_json))
}

async fn get_all_proxies(State(state): State<Arc<RwLock<Health>>>) -> Json<Vec<ProxyId>> {
    Json(state.read().await.proxies.keys().cloned().collect())
}

async fn proxy_health(
    State(state): State<Arc<RwLock<Health>>>,
    Path(proxy): Path<ProxyId>,
    auth: TypedHeader<Authorization<Basic>>
) -> Result<(StatusCode, Json<ProxyStatus>), StatusCode> {
    let Some(ref monitoring_key) = CONFIG_CENTRAL.monitoring_api_key else {
        return Err(StatusCode::NOT_IMPLEMENTED);
    };

    if auth.password() != monitoring_key {
        return Err(StatusCode::UNAUTHORIZED)
    }

    if let Some(reported_back) = state.read().await.proxies.get(&proxy) {
        if reported_back.online() {
            Err(StatusCode::OK)
        } else {
            Ok((StatusCode::SERVICE_UNAVAILABLE, Json(reported_back.clone())))
        }
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn get_control_tasks(
    State(state): State<Arc<RwLock<Health>>>,
    proxy_auth: Authorized,
) -> StatusCode {
    let proxy_id = proxy_auth.get_from().proxy_id(); 
    // Once this is freed the connection will be removed from the map of connected proxies again
    // This ensures that when the connection is dropped and therefore this response future the status of this proxy will be updated
    let _connection_remover = ConnectedGuard::connect(&proxy_id, &state).await;

    // In the future, this will wait for control tasks for the given proxy
    tokio::time::sleep(Duration::from_secs(60 * 60)).await;

    StatusCode::OK
}

struct ConnectedGuard<'a> {
    proxy: &'a ProxyId,
    state: &'a Arc<RwLock<Health>>
}

impl<'a> ConnectedGuard<'a> {
    async fn connect(proxy: &'a ProxyId, state: &'a Arc<RwLock<Health>>) -> ConnectedGuard<'a> {
        {
            state.write().await.proxies
                .entry(proxy.clone())
                .and_modify(ProxyStatus::connect)
                .or_insert(ProxyStatus::new());
        }
        Self { proxy, state }
    }
}

impl<'a> Drop for ConnectedGuard<'a> {
    fn drop(&mut self) {
        let proxy_id = self.proxy.clone();
        let map = self.state.clone();
        tokio::spawn(async move {
            // We wait here for one second to give the client a bit of time to reconnect incrementing the connection count so that it will be one again after the decrement
            tokio::time::sleep(Duration::from_secs(1)).await;
            map.write()
                .await
                .proxies
                .get_mut(&proxy_id)
                .expect("Has to exist as we don't remove items and the constructor of this type inserts the entry")
                .disconnect();
        });
    }
}
