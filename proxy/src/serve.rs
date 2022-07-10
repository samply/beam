use hyper::Client;
use hyper_tls::HttpsConnector;
use shared::config;
use tracing::{info, debug, warn, error};

use crate::{serve_health, serve_tasks};

pub(crate) async fn serve() -> anyhow::Result<()> {
    let client = Client::builder()
        .build::<_, hyper::Body>(HttpsConnector::new());

    let config = config::CONFIG_PROXY.clone();

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

    info!("Listening for requests on {}", config.bind_addr);
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

