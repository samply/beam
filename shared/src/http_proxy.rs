use std::time::Duration;

use http::Uri;
use hyper::{Client, client::{HttpConnector, connect::Connect}, service::Service};
use hyper_proxy::{Intercept, Proxy, ProxyConnector, Custom};
use hyper_tls::HttpsConnector;
use once_cell::sync::OnceCell;
use tracing::{debug, info};

use crate::{config, errors::SamplyBeamError, BeamId};

pub fn build_hyper_client(proxy_uri: Option<Uri>) -> Result<Client<ProxyConnector<HttpsConnector<HttpConnector>>>, std::io::Error> {
    let proxy = if let Some(proxy_uri) = proxy_uri {
        info!("Using proxy {}", proxy_uri);
        Proxy::new(Intercept::All, proxy_uri)
    } else {
        Proxy::new(Intercept::None, "http://bogusproxy:4223".parse().unwrap())
    };
    let mut http = HttpConnector::new();
    http.set_connect_timeout(Some(Duration::from_secs(1)));
    let proxy_connector = ProxyConnector::from_proxy(HttpsConnector::new_with_connector(http), proxy)?;
    
    Ok(Client::builder().build(proxy_connector))
}

#[cfg(test)]
mod test {

    use hyper::{Client, client::{HttpConnector, connect::Connect}, Uri, Request, body};
    use hyper_proxy::ProxyConnector;
    use hyper_tls::HttpsConnector;

    use super::build_hyper_client;

    const HTTP: &str = "http://ip-api.com/json";
    const HTTPS: &str = "https://ifconfig.me/";

    #[tokio::test]
    async fn https_without_proxy() {
        let client = build_hyper_client(None).unwrap();
        run(HTTPS.parse().unwrap(), client).await;
    }

    #[tokio::test]
    async fn http_without_proxy() {
        let client = build_hyper_client(None).unwrap();
        run(HTTP.parse().unwrap(), client).await;
    }

    #[tokio::test]
    async fn http_with_proxy() {
        let client = build_hyper_client(Some("http://localhost:3128".parse().unwrap())).unwrap();
        run(HTTP.parse().unwrap(), client).await;
    }

    #[tokio::test]
    async fn https_with_proxy() {
        let client = build_hyper_client(Some("http://localhost:3128".parse().unwrap())).unwrap();
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