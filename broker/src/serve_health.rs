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
    {
        state.write().await.proxies.insert(proxy_id.clone(), ProxyStatus::new());
    }

    // This will in the wait for control tasks for the given proxy
    tokio::time::sleep(Duration::from_secs(5* 60)).await;

    {
        state.write().await.proxies.remove(&proxy_id);
    }
    StatusCode::OK
}
