use std::{sync::Arc, time::{Duration, SystemTime}};

use axum::{extract::{State, Path}, http::StatusCode, routing::get, Json, Router, TypedHeader, headers::{Authorization, authorization::Basic}};
use serde::{Serialize, Deserialize};
use shared::{crypto_jwt::Authorized, Msg, beam_id::ProxyId, config::CONFIG_CENTRAL};
use tokio::sync::RwLock;

use crate::health::{Health, VaultStatus, Verdict, ProxyStatus};

#[derive(Serialize)]
struct HealthOutput<'a> {
    summary: Verdict,
    vault: HealthOutputVault<'a>,
}

#[derive(Serialize)]
struct HealthOutputVault<'a> {
    status: &'a str,
}

pub(crate) fn router(health: Arc<RwLock<Health>>) -> Router {
    Router::new()
        .route("/v1/health", get(handler))
        .route("/v1/health/proxies/:proxy_id", get(proxy_health))
        .route("/v1/control", get(get_control_tasks))
        .with_state(health)
}

// GET /v1/health
async fn handler<'a>(
    State(state): State<Arc<RwLock<Health>>>,
) -> (StatusCode, Json<HealthOutput<'a>>) {
    let state = state.read().await;
    let (statuscode, summary, status_vault) = match state.vault {
        VaultStatus::Ok => (StatusCode::OK, Verdict::Healthy, "ok"),
        VaultStatus::LockedOrSealed => (
            StatusCode::SERVICE_UNAVAILABLE,
            Verdict::Unhealthy,
            "sealed",
        ),
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            Verdict::Unhealthy,
            "unavailable",
        ),
    };
    let health_as_json = HealthOutput {
        summary,
        vault: HealthOutputVault {
            status: status_vault,
        },
    };
    (statuscode, Json(health_as_json))
}


async fn proxy_health(
    State(state): State<Arc<RwLock<Health>>>,
    Path(proxy): Path<ProxyId>,
    auth: TypedHeader<Authorization<Basic>>
) -> Result<Json<ProxyStatus>, StatusCode> {
    let Some(ref monitoring_key) = CONFIG_CENTRAL.monitoring_api_key else {
        return Err(StatusCode::NOT_IMPLEMENTED);
    };

    if auth.password() != monitoring_key {
        return Err(StatusCode::UNAUTHORIZED)
    }

    if let Some(proxy_status) = state.read().await.proxies.get(&proxy) {
        Ok(Json(proxy_status.clone()))
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

async fn get_control_tasks(
    State(state): State<Arc<RwLock<Health>>>,
    proxy_auth: Authorized,
) -> StatusCode {
    let proxy_id = proxy_auth.get_from().get_proxy_id(); 
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
            state.write().await.proxies.insert(proxy.clone(), ProxyStatus::new());
        }
        Self { proxy, state }
    }
}

impl<'a> Drop for ConnectedGuard<'a> {
    fn drop(&mut self) {
        let proxy_id = self.proxy.clone();
        let map = self.state.clone();
        tokio::spawn(async move {
            map.write().await.proxies.remove(&proxy_id);
        });
    }
}
