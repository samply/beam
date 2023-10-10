use std::{collections::HashSet, ops::Deref, time::Duration};

use axum::async_trait;
use http::{Request, Response, Uri};
use hyper::{
    client::{conn, connect::Connect, HttpConnector},
    service::Service,
    Body, Client,
};
use hyper_proxy::{Custom, Intercept, Proxy, ProxyConnector};
use hyper_timeout::TimeoutConnector;
use hyper_tls::{
    native_tls::{Certificate, TlsConnector},
    HttpsConnector,
};
use mz_http_proxy::hyper::connector;
use once_cell::sync::OnceCell;
use openssl::x509::X509;
use tracing::{debug, info, warn};

use crate::{config, errors::SamplyBeamError};

pub type SamplyHttpClient = Client<TimeoutConnector<ProxyConnector<HttpsConnector<HttpConnector>>>>;

pub fn build(
    ca_certificates: &Vec<X509>,
    timeout: Option<Duration>,
    keepalive: Option<Duration>,
) -> Result<SamplyHttpClient, std::io::Error> {
    let mut http = HttpConnector::new();
    http.set_connect_timeout(Some(Duration::from_secs(1)));
    http.enforce_http(false);
    http.set_keepalive(keepalive);
    let tls = build_tls_connector(ca_certificates)?;
    let https = HttpsConnector::from((http, tls.clone().into()));
    let proxy_connector = connector()
        .map_err(|e| panic!("Unable to build HTTP client: {}", e))
        .unwrap();
    let mut proxy_connector = proxy_connector.with_connector(https);
    proxy_connector.set_tls(Some(tls));

    let proxies = proxy_connector
        .proxies()
        .iter()
        .map(|p| p.uri().to_string())
        .collect::<HashSet<_>>();

    if proxies.len() == 0 && ca_certificates.len() > 0 {
        warn!("Certificates for TLS termination were provided but no proxy to use. If you want to set a proxy see https://docs.rs/mz-http-proxy/0.1.0/mz_http_proxy/#proxy-selection");
    }

    let proxies = match proxies.len() {
        0 => "no proxy".to_string(),
        1 => format!("proxy {}", proxies.iter().next().unwrap()),
        num => format!("{num} proxies {:?}", proxies),
    };
    let certs = match ca_certificates.len() {
        0 => "no trusted certificate".to_string(),
        1 => "a trusted certificate".to_string(),
        num => format!("{num} trusted certificates"),
    };
    info!("Using {proxies} and {certs} for TLS termination.");

    let mut timeout_connector = hyper_timeout::TimeoutConnector::new(proxy_connector);
    timeout_connector.set_connect_timeout(timeout);
    timeout_connector.set_read_timeout(timeout);
    timeout_connector.set_write_timeout(timeout);

    let client = Client::builder().build(timeout_connector);

    Ok(client)
}

fn build_tls_connector(ca_certificates: &Vec<X509>) -> Result<TlsConnector, std::io::Error> {
    let mut tls = TlsConnector::builder();
    for cert in ca_certificates {
        const ERR: &str = "Internal Error: Unable to convert Certificate.";
        let cert = Certificate::from_pem(&cert.to_pem().expect(ERR)).expect(ERR);
        tls.add_root_certificate(cert);
    }
    tls.build().map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::Other,
            format!(
                "Unable to build TLS Connector with custom CA certificates: {}",
                e
            ),
        )
    })
}

#[cfg(test)]
mod test {

    use std::path::{Path, PathBuf};

    use hyper::{
        body,
        client::{connect::Connect, HttpConnector},
        Client, Request, Uri,
    };
    use hyper_proxy::ProxyConnector;
    use hyper_tls::HttpsConnector;
    use openssl::x509::X509;

    use crate::http_client::{self, SamplyHttpClient};

    const HTTP: &str = "http://ip-api.com/json";
    const HTTPS: &str = "https://ifconfig.me/";

    fn get_certs() -> Vec<X509> {
        if let Ok(dir) = std::env::var("TLS_CA_CERTIFICATES_DIR") {
            let dir = PathBuf::from(dir);
            crate::crypto::load_certificates_from_dir(Some(dir)).unwrap()
        } else {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn https() {
        let client = http_client::build(&get_certs(), None, None).unwrap();
        run(HTTPS.parse().unwrap(), client).await;
    }

    #[tokio::test]
    async fn http() {
        let client = http_client::build(&get_certs(), None, None).unwrap();
        run(HTTP.parse().unwrap(), client).await;
    }

    async fn run(url: Uri, client: SamplyHttpClient) {
        let req = Request::builder()
            .uri(url)
            .body(body::Body::empty())
            .unwrap();

        let mut resp = client.request(req).await.unwrap();

        let resp_string = body::to_bytes(resp.body_mut()).await.unwrap();

        println!("=> {}\n", std::str::from_utf8(&resp_string).unwrap());
    }
}
