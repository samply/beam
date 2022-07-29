use std::time::Duration;

use http::Uri;
use hyper::{Client, client::{HttpConnector, connect::Connect}, service::Service};
use hyper_proxy::{Intercept, Proxy, ProxyConnector, Custom};
use hyper_tls::{HttpsConnector, native_tls::{TlsConnector, Certificate}};
use once_cell::sync::OnceCell;
use openssl::x509::X509;
use tracing::{debug, info};

use crate::{config, errors::SamplyBeamError, BeamId};

pub fn build_hyper_client(proxy_uri: Option<Uri>, ca_certificates: Vec<X509>) -> Result<Client<ProxyConnector<HttpsConnector<HttpConnector>>>, std::io::Error> {
    let proxy = if let Some(proxy_uri) = proxy_uri {
        info!("Using proxy {}", proxy_uri);
        Proxy::new(Intercept::All, proxy_uri)
    } else {
        Proxy::new(Intercept::None, "http://bogusproxy:4223".parse().unwrap())
    };
    let mut http = HttpConnector::new();
    http.enforce_http(false);
    http.set_connect_timeout(Some(Duration::from_secs(1)));
    let mut proxy_connector = ProxyConnector::from_proxy(HttpsConnector::new_with_connector(http), proxy)?;
    if ! ca_certificates.is_empty() {
        let mut tls = TlsConnector::builder();
        for cert in ca_certificates {
            const ERR: &str = "Internal Error: Unable to convert Certificate.";
            let cert = Certificate::from_pem(&cert.to_pem().expect(ERR)).expect(ERR);
            tls.add_root_certificate(cert);
        }
        let tls = tls
            .build()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("Unable to build TLS Connector with custom CA certificates: {}", e)))?;
        proxy_connector.set_tls(Some(tls));
    }
    
    Ok(Client::builder().build(proxy_connector))
}

#[cfg(test)]
mod test {

    use std::path::{Path, PathBuf};

    use hyper::{Client, client::{HttpConnector, connect::Connect}, Uri, Request, body};
    use hyper_proxy::ProxyConnector;
    use hyper_tls::HttpsConnector;
    use openssl::x509::X509;

    use super::build_hyper_client;

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
    async fn https_without_proxy() {
        let client = build_hyper_client(None, get_certs()).unwrap();
        run(HTTPS.parse().unwrap(), client).await;
    }

    #[tokio::test]
    async fn http_without_proxy() {
        let client = build_hyper_client(None, get_certs()).unwrap();
        run(HTTP.parse().unwrap(), client).await;
    }

    #[tokio::test]
    async fn http_with_proxy() {
        let client = build_hyper_client(Some("http://localhost:3128".parse().unwrap()), get_certs()).unwrap();
        run(HTTP.parse().unwrap(), client).await;
    }

    #[tokio::test]
    async fn https_with_proxy() {
        let client = build_hyper_client(Some("http://localhost:3128".parse().unwrap()), get_certs()).unwrap();
        run(HTTPS.parse().unwrap(), client).await;
    }

    async fn run(url: Uri, client: Client<impl Connect + Clone + Send + Sync + 'static>) {
        let req = Request::builder()
            .uri(url)
            .body(body::Body::empty())
            .unwrap();

        let mut resp = client.request(req).await.unwrap();

        let resp_string = body::to_bytes(resp.body_mut()).await.unwrap();

        println!("=> {}\n", std::str::from_utf8(&resp_string).unwrap());
    }
}