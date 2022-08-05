use std::fmt::Write;

use hyper::Client;
use shared::{config, errors::SamplyBeamError};
use tracing::{info, debug, warn, error};

use crate::{serve_health, serve_tasks};

pub(crate) async fn serve() -> anyhow::Result<()> {
    let config = config::CONFIG_PROXY.clone();
    
    let client = shared::http_proxy::build_hyper_client(config.tls_ca_certificates)
        .map_err(SamplyBeamError::HttpProxyProblem)?;

    let router_tasks = serve_tasks::router(&client);

    let router_health = serve_health::router();
    
    let app = 
        router_tasks
        .merge(router_health)
        .layer(axum::middleware::from_fn(shared::middleware::log));

    // Graceful shutdown handling
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    ctrlc::set_handler(move || {
        if tx.blocking_send(()).is_err() {
            warn!("Unable to send signal for clean shutdown... ignoring.");
        }
        })
        .expect("Error setting handler for graceful shutdown.");

    let mut apps_joined = String::new();
    config.api_keys.keys().for_each(|k| write!(apps_joined, "{} ", k.to_string().split('.').next().unwrap()).unwrap());
    info!("This is Proxy {} listening on {}. {} apps are known: {}", config.proxy_id, config.bind_addr, config.api_keys.len(), apps_joined);
    
    axum::Server::bind(&config.bind_addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(graceful_waiter(rx))
        .await?;

    Ok(())
}

async fn graceful_waiter(mut rx: tokio::sync::mpsc::Receiver<()>) {
    rx.recv().await;
    info!("Shutting down gracefully.");
}

