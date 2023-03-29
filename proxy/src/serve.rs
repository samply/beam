use std::{fmt::Write, net::SocketAddr};

use hyper::{Client, client::HttpConnector, header};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use shared::{config, errors::SamplyBeamError, config_shared, config_proxy, http_client::SamplyHttpClient};
use tracing::{info, debug, warn, error};

use crate::{serve_health, serve_tasks, banner, monitor};

pub(crate) async fn serve(config: config_proxy::Config, client: SamplyHttpClient) -> anyhow::Result<()> {
    let router_tasks = serve_tasks::router(&client);

    let router_health = serve_health::router();

    let router_monitorer = monitor::router();
    
    let app = router_tasks
        .merge(router_health)
        .merge(router_monitorer)
        .layer(axum::middleware::from_fn(monitor::monitor_and_log))
        .layer(axum::middleware::map_response(banner::set_server_header));

    let mut apps_joined = String::new();
    config.api_keys.keys().for_each(|k| write!(apps_joined, "{} ", k.to_string().split('.').next().unwrap()).unwrap());
    info!("Startup complete. This is Proxy {} listening on {}. {} apps are known: {}", config.proxy_id, config.bind_addr, config.api_keys.len(), apps_joined);
    
    axum::Server::bind(&config.bind_addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shared::graceful_shutdown::wait_for_signal())
        .await?;

    Ok(())
}
