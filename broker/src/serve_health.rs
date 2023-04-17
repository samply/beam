use std::sync::Arc;

use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
use hyper::{HeaderMap, header};
use serde::Serialize;
use shared::compare_client_server_version::{compare_version, Verdict::*};
use tokio::sync::RwLock;
use tracing::{warn, debug, error};

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
    headers: HeaderMap,
    State(state): State<Arc<RwLock<Health>>>
) -> (StatusCode, Json<HealthOutput<'a>>) {
    if let Some(their_version_header) = headers.get(header::USER_AGENT) {
        match compare_version(their_version_header) {
            BeamWithMatchingVersion | NotBeam => {
                // we're happy
            },
            BeamWithMismatchingVersion(their_ver) => {
                warn!("Beam.Proxy has mismatching version: {their_ver}");
            },
            BeamWithInvalidVersion(their_ver) => {
                error!("Beam.Proxy has invalid version: {their_ver}");
            },
        }
    }
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
