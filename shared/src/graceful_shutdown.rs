use std::time::Duration;

use tokio::signal::unix::{signal,SignalKind};
use tracing::info;

pub async fn wait_for_signal() {
    let mut sigterm = signal(SignalKind::terminate()).unwrap();
    let mut sigkill = signal(SignalKind::interrupt()).unwrap();
    let signal = tokio::select! {
        _ = sigterm.recv() => "SIGTERM",
        _ = sigkill.recv() => "SIGKILL"
    };
    // The following does not print in docker-compose setups but it does when run individually.
    // Probably a docker-compose error.
    info!("Received signal ({signal}) - shutting down gracefully.");
}