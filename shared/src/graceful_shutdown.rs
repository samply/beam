use std::time::Duration;

use tokio::signal::unix::{signal, SignalKind};
use tracing::info;

#[cfg(unix)]
pub async fn wait_for_signal() {
    let mut sigterm = signal(SignalKind::terminate())
        .expect("Unable to register shutdown handler; are you running a Unix-based OS?");
    let mut sigint = signal(SignalKind::interrupt())
        .expect("Unable to register shutdown handler; are you running a Unix-based OS?");
    let signal = tokio::select! {
        _ = sigterm.recv() => "SIGTERM",
        _ = sigint.recv() => "SIGINT"
    };
    // The following does not print in docker-compose setups but it does when run individually.
    // Probably a docker-compose error.
    info!("Received signal ({signal}) - shutting down gracefully.");
}

#[cfg(windows)]
pub async fn wait_for_signal() {
    if let Err(e) = tokio::signal::ctrl_c().await {
        panic!("Unable to register shutdown handler: {e}.");
    }
    info!("Received shutdown signal - shutting down gracefully.");
}
