use http::Uri;
use hyper::{Client, client::{HttpConnector, connect::Connect}, service::Service};
use hyper_proxy::{Intercept, Proxy, ProxyConnector};
use tracing::{debug, info};

use crate::{config, errors::SamplyBrokerError, BeamIdTrait};

// TODO: To access http (non-TLS) targets, implement http_headers, cf. documentation of hyper_proxy

pub fn build_hyper_client() -> Result<Client<ProxyConnector<HttpConnector>>, std::io::Error> {
    let id = BeamIdTrait::AppId("somestring");
    let http = HttpConnector::new();
    let mut proxy_connector = ProxyConnector::new(http)?;
    if let Some(proxy_uri) = &config::CONFIG_SHARED.http_proxy {
        proxy_connector.add_proxy(Proxy::new(Intercept::All, proxy_uri.clone()));
        info!("Using proxy {}", proxy_uri);
    }
    
    Ok(Client::builder().build(proxy_connector))
}