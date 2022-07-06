use std::time::SystemTime;

use axum::{
    extract::{Extension},
    http::{uri::Uri, Request, Response, HeaderValue},
    routing::any,
    Router
};
use httpdate::{fmt_http_date, HttpDate};
use hyper::{Body, StatusCode, header::{self, HeaderName}, Client, client::HttpConnector, body, Method};
use hyper_tls::HttpsConnector;
use serde_json::Value;
use shared::{crypto_jwt, errors::SamplyBrokerError};
use tracing::{info, debug, warn, error};

use crate::{auth::AuthenticatedProxyClient, config::Config};

pub(crate) async fn reverse_proxy(
    config: Config
) -> anyhow::Result<()> {
    let client = Client::builder()
        .build::<_, hyper::Body>(HttpsConnector::new());

    let app = Router::new()
        .route("/*path", any(handler))
        .layer(Extension(client))
        .layer(Extension(config.clone()));

    // Graceful shutdown handling
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    ctrlc::set_handler(move || {
            if let Err(_) = tx.blocking_send(()) {
                warn!("Unable to send signal for clean shutdown... ignoring.");
            }
        })
        .expect("Error setting handler for graceful shutdown.");

    info!("Listening for requests on {}", config.bind_addr);
    axum::Server::bind(&config.bind_addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(graceful_waiter(rx))
        .await
        .unwrap();

    Ok(())
}

async fn graceful_waiter(mut rx: tokio::sync::mpsc::Receiver<()>) {
    rx.recv().await;
    info!("Shutting down gracefully.");
}

async fn handler(
    Extension(client): Extension<Client<HttpsConnector<HttpConnector>>>,
    Extension(config): Extension<Config>,
    AuthenticatedProxyClient(auth_client): AuthenticatedProxyClient,
    mut req: Request<Body>,
) -> Result<Response<Body>,(StatusCode, &'static str)> {
    const ERR_BODY: (StatusCode, &'static str) = (StatusCode::BAD_REQUEST, "Invalid body");
    const ERR_INTERNALCRYPTO: (StatusCode, &'static str) = (StatusCode::INTERNAL_SERVER_ERROR, "Cryptography failed; see server logs.");

    debug!(?req, ?auth_client, "<=");

    let path = req.uri().path();
    let path_query = req
        .uri()
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(path);

    let target_uri = Uri::try_from(config.broker_uri.to_string() + path_query.trim_start_matches('/'))
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid path queried."))?;

    let body_ref = req.body_mut();
    let body = body::to_bytes(body_ref).await
        .map_err(|_| ERR_BODY)?;
    let body = String::from_utf8(body.to_vec())
        .map_err(|_| ERR_BODY)?;

    let mut body = if body.is_empty() {
        debug!("Body is empty, substituting empty json.");
        serde_json::from_str::<Value>("{}").unwrap()
    } else {
        match serde_json::from_str::<Value>(&body) {
            Ok(val) => {
                debug!("Body is valid json");
                val
            },
            Err(e) => {
                warn!("Received Body is invalid json: {}", e);
                return Err(ERR_BODY);
            }
        }
    };

    let privkey_pem = &crate::config::CONFIG.privkey_pem;
    // Sign non-extended JSON
    let token_without_extended_signature = crypto_jwt::sign_to_jwt(&body, privkey_pem, &config.client_id).await
        .map_err(|e| {
            error!("Crypto failed: {}", e);
            ERR_INTERNALCRYPTO
        })?;

    // Create and sign extended JSON
    let headers_mut = req.headers_mut();
    headers_mut.insert(header::DATE, HeaderValue::from_str(&fmt_http_date(SystemTime::now())).expect("Internal error: Unable to format system time"));

    let digest = crypto_jwt::make_extra_fields_digest(req.method(), req.uri(), req.headers())
        .map_err(|_| ERR_INTERNALCRYPTO)?;

    body.as_object_mut().unwrap().insert("extra_fields_digest".to_string(), Value::String(digest));

    let token_with_extended_signature = crypto_jwt::sign_to_jwt(&body, privkey_pem, &config.client_id).await
        .map_err(|e| {
            error!("Crypto failed: {}", e);
            ERR_INTERNALCRYPTO
        })?;
    
    let length = token_without_extended_signature.len();
    *req.body_mut() = token_without_extended_signature.into();
    let mut auth_header = String::from("SamplyJWT ");
    auth_header.push_str(&token_with_extended_signature);

    *req.uri_mut() = target_uri;
    let headers_mut = req.headers_mut();
    headers_mut.insert(header::HOST, config.broker_host_header);
    headers_mut.insert(header::CONTENT_LENGTH, length.into());
    headers_mut.insert(header::CONTENT_TYPE, HeaderValue::from_str("application/jwt").unwrap());
    headers_mut.insert(header::AUTHORIZATION, HeaderValue::from_str(&auth_header).unwrap());
    headers_mut.remove(header::PROXY_AUTHORIZATION);

    info!("=> {:?}", req);
    
    let a = client.request(req).await
        .map_err(|e| {
            warn!("Request to broker failed: {}", e.to_string());
            (StatusCode::BAD_GATEWAY, "Upstream error; see server logs.")
        })?;
    
    Ok(a)
}