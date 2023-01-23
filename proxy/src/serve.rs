use std::fmt::Write;

use hyper::{Client, client::HttpConnector};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use shared::{config, errors::SamplyBeamError, config_shared, config_proxy, http_proxy::SamplyHttpClient};
use tracing::{info, debug, warn, error};

use crate::{serve_health, serve_tasks};

pub(crate) async fn serve(config: config_proxy::Config, client: SamplyHttpClient) -> anyhow::Result<()> {
    let router_tasks = serve_tasks::router(&client);

    let router_health = serve_health::router();
    
    let app = 
        router_tasks
        .merge(router_health)
        .layer(axum::middleware::from_fn(shared::middleware::log));

    // Graceful shutdown handling
    let (shutdown_tx, shutdown_rx) = tokio::sync::mpsc::channel(1);

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await
            .expect("Unable to listen for Ctrl+C for graceful shutdown");
        if shutdown_tx.blocking_send(()).is_err() {
            warn!("Unable to send signal for clean shutdown... ignoring.");
        }
    });

    let mut apps_joined = String::new();
    config.api_keys.keys().for_each(|k| write!(apps_joined, "{} ", k.to_string().split('.').next().unwrap()).unwrap());
    info!("This is Proxy {} listening on {}. {} apps are known: {}", config.proxy_id, config.bind_addr, config.api_keys.len(), apps_joined);
    
    axum::Server::bind(&config.bind_addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(graceful_waiter(shutdown_rx))
        .await?;

    Ok(())
}

async fn graceful_waiter(mut rx: tokio::sync::mpsc::Receiver<()>) {
    rx.recv().await;
    info!("Shutting down gracefully.");
}

