use http::Uri;
use hyper::{Client, client::{HttpConnector, connect::Connect}, service::Service};
use hyper_proxy::{Intercept, Proxy, ProxyConnector};
use tracing::{debug, info};

use crate::{config, errors::SamplyBeamError, BeamId};

pub fn build_hyper_client() -> Result<Client<ProxyConnector<HttpConnector>>, std::io::Error> {
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    let mut proxy_connector = ProxyConnector::new(http)?;
    if let Some(proxy_uri) = &config::CONFIG_SHARED.http_proxy {
        proxy_connector.add_proxy(Proxy::new(Intercept::All, proxy_uri.clone()));
        info!("Using proxy {}", proxy_uri);
    }
    
    Ok(Client::builder().build(proxy_connector))
}