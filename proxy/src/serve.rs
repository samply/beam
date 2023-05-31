use std::{fmt::Write, net::SocketAddr};

use hyper::{client::HttpConnector, header, Client};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use shared::{
    config, config_proxy, config_shared, errors::SamplyBeamError, http_client::SamplyHttpClient,
};
use tracing::{debug, error, info, warn};

use crate::{banner, serve_health, serve_tasks};

pub(crate) async fn serve(
    config: config_proxy::Config,
    client: SamplyHttpClient,
) -> anyhow::Result<()> {
    let router_tasks = serve_tasks::router(&client);

    let router_health = serve_health::router();

    let app = router_tasks.merge(router_health);

    #[cfg(feature = "sockets")]
    let app = app.merge(crate::serve_sockets::router(client));
    // Middleware needs to be set last
    let app = app
        .layer(axum::middleware::from_fn(shared::middleware::log))
        .layer(axum::middleware::map_response(banner::set_server_header));

    let mut apps_joined = String::new();
    config.api_keys.keys().for_each(|k| {
        write!(apps_joined, "{} ", k.to_string().split('.').next().unwrap()).unwrap()
    });
    info!(
        "Startup complete. This is Proxy {} listening on {}. {} apps are known: {}",
        config.proxy_id,
        config.bind_addr,
        config.api_keys.len(),
        apps_joined
    );

    axum::Server::bind(&config.bind_addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shared::graceful_shutdown::wait_for_signal())
        .await?;

    Ok(())
}
