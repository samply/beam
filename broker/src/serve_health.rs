use std::sync::Arc;

use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use serde::Serialize;
use tokio::sync::RwLock;

use crate::health::{Health, VaultStatus, Verdict};

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
        .with_state(health)
}

// GET /v1/health
async fn handler<'a>(
    State(state): State<Arc<RwLock<Health>>>
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
