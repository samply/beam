#![allow(unused_imports)]

use std::time::Duration;

use backoff::{future::retry_notify, ExponentialBackoff};
use hyper::header;
use hyper::{body, client::HttpConnector, Client, Method, Request, StatusCode, Uri};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use beam_lib::AppOrProxyId;
use shared::{PlainMessage, MsgEmpty, EncryptedMessage, is_actually_hyper_timeout};
use shared::crypto::CryptoPublicPortion;
use shared::errors::SamplyBeamError;
use shared::http_client::{self, SamplyHttpClient};
use shared::{config, config_proxy::Config};
use tracing::{debug, error, info, warn};

use crate::serve_tasks::sign_request;

mod auth;
mod banner;
mod crypto;
mod serve;
mod serve_health;
mod serve_tasks;
#[cfg(feature = "sockets")]
mod serve_sockets;

pub(crate) const PROXY_TIMEOUT: u64 = 120;

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    shared::config::prepare_env();
    shared::logger::init_logger()?;
    banner::print_banner();

    let config = config::CONFIG_PROXY.clone();
    let client = http_client::build(
        &config::CONFIG_SHARED.tls_ca_certificates,
        Some(Duration::from_secs(PROXY_TIMEOUT)),
        Some(Duration::from_secs(20)),
    )
    .map_err(SamplyBeamError::HttpProxyProblem)?;

    if let Err(err) = retry_notify(
        ExponentialBackoff::default(),
        || async { Ok(get_broker_health(&config, &client).await?) },
        |err, dur: Duration| {
            warn!(
                "Still trying to reach Broker: {}. Retrying in {}s",
                err,
                dur.as_secs()
            );
        },
    )
    .await
    {
        error!("Giving up reaching Broker: {}", err);
        std::process::exit(1);
    } else {
        info!("Connected to Broker: {}", &config.broker_uri);
    }

    if let Err(err) = retry_notify(
        ExponentialBackoff::default(),
        || async { Ok(init_crypto(config.clone(), client.clone()).await?) },
        |err, dur: Duration| {
            warn!(
                "Still trying to initialize certificate chain: {}. Retrying in {}s",
                err,
                dur.as_secs()
            );
        },
    )
    .await
    {
        error!("Giving up on initializing certificate chain: {}", err);
        std::process::exit(1);
    } else {
        debug!("Certificate chain successfully initialized and validated");
    }
    spawn_controller_polling(client.clone(), config.clone());

    serve::serve(config, client).await?;
    Ok(())
}

async fn init_crypto(config: Config, client: SamplyHttpClient) -> Result<(), SamplyBeamError> {
    let private_crypto_proxy = shared::config_shared::load_private_crypto_for_proxy()?;
    shared::crypto::init_cert_getter(crypto::build_cert_getter(
        config.clone(),
        client.clone(),
        private_crypto_proxy.clone(),
    )?);
    shared::crypto::init_ca_chain().await?;

    let _public_info: Vec<_> =
        shared::crypto::get_all_certs_and_clients_by_cname_as_pemstr(&config.proxy_id)
            .await
            .into_iter()
            .filter_map(|r| {
                r.map_err(|e| debug!("Unable to fetch certificate: {e}"))
                    .ok()
            })
            .collect();
    let (serial, cname) =
        shared::config_shared::init_public_crypto_for_proxy(private_crypto_proxy).await?;
    if cname != config.proxy_id.to_string() {
        return Err(SamplyBeamError::ConfigurationFailed(format!("Unable to retrieve a certificate matching your Proxy ID. Expected {}, got {}. Please check your configuration", cname, config.proxy_id.to_string())));
    }

    info!("Certificate retrieved for our proxy ID {cname} (serial {serial})");

    Ok(())
}

async fn get_broker_health(
    config: &Config,
    client: &SamplyHttpClient,
) -> Result<(), SamplyBeamError> {
    let uri = Uri::builder()
        .scheme(
            config
                .broker_uri
                .scheme()
                .expect("Config broker uri to have valid scheme")
                .as_str(),
        )
        .authority(
            config
                .broker_uri
                .authority()
                .expect("Config broker uri to have valid authority")
                .to_owned(),
        )
        .path_and_query("/v1/health")
        .build()
        .map_err(|e| SamplyBeamError::HttpRequestBuildError(e))
        .expect("Uri to be constructed correctly");

    let resp = retry_notify(
        backoff::ExponentialBackoffBuilder::default()
            .with_max_interval(Duration::from_secs(10))
            .with_max_elapsed_time(Some(Duration::from_secs(30)))
            .build(),
        || async {
            let req = Request::builder()
                .method(Method::GET)
                .uri(&uri)
                .header(hyper::header::USER_AGENT, env!("SAMPLY_USER_AGENT"))
                .body(body::Body::empty())
                .expect("Request to be constructed correctly");
            Ok(client.request(req).await?)
        },
        |err, b: Duration| {
            warn!(
                "Unable to connect to Broker: {}. Retrying in {}s",
                err,
                b.as_secs()
            );
        },
    )
    .await?;

    match resp.status() {
        StatusCode::OK => Ok(()),
        _ => Err(SamplyBeamError::InternalSynchronizationError(format!(
            "Unexpected reply from Broker, received status code {}",
            resp.status()
        ))),
    }
}

fn spawn_controller_polling(client: SamplyHttpClient, config: Config) {
    const RETRY_INTERVAL: Duration = Duration::from_secs(60);
    tokio::spawn(async move {
        loop {
            let body = EncryptedMessage::MsgEmpty(MsgEmpty {
                from: AppOrProxyId::Proxy(config.proxy_id.clone()),
            });
            let (parts, body) = Request::get(format!("{}v1/control", config.broker_uri))
                .header(header::USER_AGENT, env!("SAMPLY_USER_AGENT"))
                .body(body)
                .expect("To build request successfully")
                .into_parts();

            let req = sign_request(body, parts, &config, None).await.expect("Unable to sign request; this should always work");
            // In the future this will poll actual control related tasks
            match client.request(req).await {
                Ok(res) => {
                    match res.status() {
                        StatusCode::OK => {
                            // Process control task
                        },
                        other => {
                            warn!("Got unexpected status getting control tasks from broker: {other}");
                            tokio::time::sleep(RETRY_INTERVAL).await;
                        }
                    };
                },
                // For some reason e.is_timeout() does not work
                Err(e) if is_actually_hyper_timeout(&e) => {
                    debug!("Connection to broker timed out; retrying: {e}");
                },
                Err(e) => {
                    warn!("Error getting control tasks from broker; retrying in {}s: {e}", RETRY_INTERVAL.as_secs());
                    tokio::time::sleep(RETRY_INTERVAL).await;
                }
            };
        }
    });
}
