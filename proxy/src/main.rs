#![allow(unused_imports)]

use std::future::Future;
use std::time::Duration;

use axum::http::{header, HeaderValue, StatusCode};
use beam_lib::AppOrProxyId;
use futures::future::Ready;
use shared::{reqwest, EncryptedMessage, MsgEmpty, PlainMessage};
use shared::crypto::CryptoPublicPortion;
use shared::errors::SamplyBeamError;
use shared::http_client::{self, SamplyHttpClient};
use shared::{config, config_proxy::Config};
use tokio::time::Instant;
use tracing::{debug, error, info, warn};
use tryhard::{backoff_strategies::ExponentialBackoff, RetryFuture, RetryFutureConfig};

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
    shared::logger::init_logger()?;
    banner::print_banner();

    let config = config::CONFIG_PROXY.clone();
    let client = http_client::build(
        &config::CONFIG_SHARED.tls_ca_certificates,
        Some(Duration::from_secs(PROXY_TIMEOUT)),
        Some(Duration::from_secs(20)),
    )?;

    if let Err(err) = retry_notify(|| get_broker_health(&config, &client), |err, dur| {
        warn!("Still trying to reach Broker: {err}. Retrying in {}s", dur.as_secs());
    }).await {
        error!("Giving up reaching Broker: {err}");
        std::process::exit(1);
    } else {
        info!("Connected to Broker: {}", &config.broker_uri);
    }

    if let Err(err) = retry_notify(|| init_crypto(config.clone(), client.clone()), |err, dur| {
        warn!("Still trying to initialize certificate chain: {err}. Retrying in {}s", dur.as_secs());
    }).await {
        error!("Giving up on initializing certificate chain: {}", err);
        std::process::exit(1);
    } else {
        debug!("Certificate chain successfully initialized and validated");
    }
    spawn_controller_polling(client.clone(), config.clone());

    serve::serve(config, client).await?;
    Ok(())
}

fn retry_notify<F, T, Fut, E, Cb>(f: F, on_error: Cb) -> RetryFuture<F, Fut, ExponentialBackoff, Box<dyn Fn(u32, Option<Duration>, &E) -> Ready<()>>>
where 
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    Cb: Fn(&E, Duration) + 'static,
    
{
    tryhard::retry_fn(f)
        .retries(100)
        .exponential_backoff(Duration::from_secs(1))
        .max_delay(Duration::from_secs(120))
        .on_retry(Box::new(move |_, b, e| futures::future::ready(on_error(e, b.unwrap_or(Duration::MAX)))))
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
    let uri = config.broker_uri
        .join("/v1/health")
        .expect("Uri to be constructed correctly");
    let resp = client
        .get(uri.clone())
        .header(header::USER_AGENT, HeaderValue::from_static(env!("SAMPLY_USER_AGENT")))
        .send()
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
        let mut retries_this_min = 0;
        let mut reset_interval = std::pin::pin!(tokio::time::sleep(Duration::from_secs(60)));
        loop {
            let body = EncryptedMessage::MsgEmpty(MsgEmpty {
                from: AppOrProxyId::Proxy(config.proxy_id.clone()),
            });
            let (parts, body) = axum::http::Request::get(format!("{}v1/control", config.broker_uri))
                .header(header::USER_AGENT, env!("SAMPLY_USER_AGENT"))
                .body(body)
                .expect("To build request successfully")
                .into_parts();

            let req = sign_request(body, parts, &config, None).await.expect("Unable to sign request; this should always work");
            // In the future this will poll actual control related tasks
            match client.execute(req).await {
                Ok(res) => {
                    match res.status() {
                        StatusCode::OK => {
                            // Process control task
                        },
                        status @ (StatusCode::GATEWAY_TIMEOUT | StatusCode::BAD_GATEWAY) => {
                            if retries_this_min < 10 {
                                retries_this_min += 1;
                                debug!("Connection to broker timed out; retrying.");
                            } else {
                                warn!("Retried more then 10 times in one minute getting status code: {status}");
                                tokio::time::sleep(RETRY_INTERVAL).await;
                                continue;
                            }
                        },
                        other => {
                            warn!("Got unexpected status getting control tasks from broker: {other}");
                            tokio::time::sleep(RETRY_INTERVAL).await;
                        }
                    };
                },
                Err(e) if e.is_timeout() => {
                    debug!("Connection to broker timed out; retrying: {e}");
                },
                Err(e) => {
                    warn!("Error getting control tasks from broker; retrying in {}s: {e}", RETRY_INTERVAL.as_secs());
                    tokio::time::sleep(RETRY_INTERVAL).await;
                }
            };
            if reset_interval.is_elapsed() {
                retries_this_min = 0;
                reset_interval.as_mut().reset(Instant::now() + Duration::from_secs(60));
            }
        }
    });
}
