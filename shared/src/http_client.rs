use std::{collections::HashSet, ops::Deref, time::Duration};

use axum::async_trait;
use axum::http::{Request, Response, Uri};
use itertools::Itertools;
use once_cell::sync::OnceCell;
use openssl::x509::X509;
use reqwest::{Certificate, Client, ClientBuilder};
use tracing::{debug, info, warn};

use crate::{config, errors::SamplyBeamError};

pub type SamplyHttpClient = reqwest::Client;

pub fn build(
    ca_certificates: &Vec<Certificate>,
    timeout: Option<Duration>,
    keepalive: Option<Duration>,
) -> Result<SamplyHttpClient, SamplyBeamError> {
    let mut builder = Client::builder().tcp_keepalive(keepalive);
    if let Some(to) = timeout {
        builder = builder.connect_timeout(to);
    }
    for cert in ca_certificates {
        builder = builder.add_root_certificate(cert.clone());
    }

    // This is not doing the logic that reqwest does ofc. reqwest supports all proxy env config vars in upper and lower case.
    // This is just for display purposes as reqwest does not expose which proxies it loaded.
    let proxies = std::env::vars()
        .filter(|(k, _)| k.to_ascii_lowercase().contains("proxy"))
        .collect::<Vec<_>>();

    if proxies.len() == 0 && ca_certificates.len() > 0 {
        warn!("Certificates for TLS termination were provided but no proxy to use. If you want to set a proxy see https://docs.rs/reqwest/#proxies");
    }

    let proxies = proxies.into_iter().map(|(k, v)| format!("{k}={v}")).join(", ");

    let certs = match ca_certificates.len() {
        0 => "no trusted certificate".to_string(),
        1 => "a trusted certificate".to_string(),
        num => format!("{num} trusted certificates"),
    };
    info!("Using {proxies} and {certs} for TLS termination.");

    builder.build().map_err(|e| SamplyBeamError::ConfigurationFailed(e.to_string()))
}

#[cfg(test)]
mod test {

    use std::path::{Path, PathBuf};

    use reqwest::{Request, Url};

    use crate::{http_client::{self, SamplyHttpClient}};

    const HTTP: &str = "http://ip-api.com/json";
    const HTTPS: &str = "https://ifconfig.me/";

    #[tokio::test]
    async fn https() {
        let client = http_client::build(&vec![], None, None).unwrap();
        run(HTTPS.parse().unwrap(), client).await;
    }

    #[tokio::test]
    async fn http() {
        let client = http_client::build(&vec![], None, None).unwrap();
        run(HTTP.parse().unwrap(), client).await;
    }

    async fn run(url: Url, client: SamplyHttpClient) {
        let resp = client.get(url).send().await.unwrap();

        println!("=> {}\n", resp.text().await.unwrap());
    }
}
