use std::{collections::HashMap, convert::Infallible, marker::PhantomData, sync::Arc, time::{Duration, SystemTime}};

use axum::{extract::{Path, State}, http::StatusCode, response::{sse::{Event, KeepAlive}, Response, Sse}, routing::get, Json, Router};
use axum_extra::{headers::{authorization::Basic, Authorization}, TypedHeader};
use beam_lib::ProxyId;
use futures_core::Stream;
use serde::{Serialize, Deserialize};
use shared::{crypto_jwt::Authorized, Msg, config::CONFIG_CENTRAL};
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};
use tracing::info;

use crate::compare_client_server_version::log_version_mismatch;

#[derive(Serialize)]
struct HealthOutput {
    summary: Verdict,
    vault: VaultStatus,
    init_status: InitStatus
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Healthy,
    Unhealthy,
    Unknown,
}

impl Default for Verdict {
    fn default() -> Self {
        Verdict::Unknown
    }
}

#[derive(Debug, Serialize, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum VaultStatus {
    Ok,
    #[default]
    Unknown,
    OtherError,
    LockedOrSealed,
    Unreachable,
}

#[derive(Debug, Serialize, Clone, Copy, Default)]
#[serde(rename_all = "lowercase")]
pub enum InitStatus {
    #[default]
    Unknown,
    FetchingIntermediateCert,
    Done
}

#[derive(Debug, Default)]
pub struct Health {
    pub vault: VaultStatus,
    pub initstatus: InitStatus,
    proxies: HashMap<ProxyId, ProxyStatus>
}

#[derive(Debug, Clone, Default)]
struct ProxyStatus {
    online_guard: Arc<Mutex<Option<SystemTime>>>
}

impl ProxyStatus {
    pub fn is_online(&self) -> bool {
        self.online_guard.try_lock().is_err()
    }
}

pub(crate) fn router(health: Arc<RwLock<Health>>) -> Router {
    Router::new()
        .route("/v1/health", get(handler))
        .route("/v1/health/proxies/{proxy_id}", get(proxy_health))
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
    Json(state.read().await.proxies.iter().filter(|(_, v)| v.is_online()).map(|(k, _)| k).cloned().collect())
}

async fn proxy_health(
    State(state): State<Arc<RwLock<Health>>>,
    Path(proxy): Path<ProxyId>,
    auth: TypedHeader<Authorization<Basic>>
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let Some(ref monitoring_key) = CONFIG_CENTRAL.monitoring_api_key else {
        return Err(StatusCode::NOT_IMPLEMENTED);
    };

    if auth.password() != monitoring_key {
        return Err(StatusCode::UNAUTHORIZED)
    }

    if let Some(reported_back) = state.read().await.proxies.get(&proxy) {
        if let Ok(last_disconnect) = reported_back.online_guard.try_lock().as_deref().copied() {
            Ok((StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({
                "last_disconnect": last_disconnect
            }))))
        } else {
            Err(StatusCode::OK)
        }
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

async fn get_control_tasks(
    State(state): State<Arc<RwLock<Health>>>,
    proxy_auth: Authorized,
) -> Result<Sse<ForeverStream>, StatusCode> {
    let proxy_id = proxy_auth.get_from().proxy_id(); 
    // Once this is freed the connection will be removed from the map of connected proxies again
    // This ensures that when the connection is dropped and therefore this response future the status of this proxy will be updated
    let status_mutex = state
        .write()
        .await
        .proxies
        .entry(proxy_id)
        .or_default()
        .online_guard
        .clone();
    let Ok(connect_guard) = tokio::time::timeout(Duration::from_secs(60), status_mutex.lock_owned()).await
    else {
        info!("Double connection!");
        return Err(StatusCode::OK);
    };

    Ok(Sse::new(ForeverStream(connect_guard)).keep_alive(KeepAlive::new()))
}

struct ForeverStream(OwnedMutexGuard<Option<SystemTime>>);

impl Stream for ForeverStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(self: std::pin::Pin<&mut Self>, _cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Self::Item>> {
        std::task::Poll::Pending
    }
}

impl Drop for ForeverStream {
    fn drop(&mut self) {
        *self.0 = Some(SystemTime::now());
    }
}