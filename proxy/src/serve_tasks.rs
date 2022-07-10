use std::time::SystemTime;

use axum::{Router, Extension, routing::any, response::Response, http::HeaderValue};
use httpdate::fmt_http_date;
use hyper::{Client, client::HttpConnector, StatusCode, Request, Body, Uri, body, header};
use hyper_proxy::ProxyConnector;
use serde::de::DeserializeOwned;
use serde_json::Value;
use shared::{config, config_proxy, crypto_jwt, MsgTaskRequest, MsgTaskResult, MsgEmpty, Msg, MsgSigned};
use tracing::{warn, debug, error};

use crate::auth::AuthenticatedProxyClient;

pub(crate) fn router(client: &Client<ProxyConnector<HttpConnector>>) -> Router {
    let config = config::CONFIG_PROXY.clone();
    Router::new()
        .route("/v1/tasks/*path", any(handler_tasks))
        .layer(Extension(client.clone()))
        .layer(Extension(config))
}

const ERR_BODY: (StatusCode, &str) = (StatusCode::BAD_REQUEST, "Invalid body");
const ERR_INTERNALCRYPTO: (StatusCode, &str) = (StatusCode::INTERNAL_SERVER_ERROR, "Cryptography failed; see server logs.");
const ERR_UPSTREAM: (StatusCode, &str) = (StatusCode::BAD_GATEWAY, "Unable to parse server's reply.");
const ERR_VALIDATION: (StatusCode, &str) = (StatusCode::BAD_GATEWAY, "Unable to verify signature in server reply.");

async fn handler_tasks(
    Extension(client): Extension<Client<ProxyConnector<HttpConnector>>>,
    Extension(config): Extension<config_proxy::Config>,
    AuthenticatedProxyClient(_): AuthenticatedProxyClient,
    req: Request<Body>,
) -> Result<Response<Body>,(StatusCode, &'static str)> {
    let path = req.uri().path();
    let path_query = req
        .uri()
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(path);

    let target_uri = Uri::try_from(config.broker_uri.to_string() + path_query.trim_start_matches('/'))
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid path queried."))?;

    let req = sign_request(req, &config, &target_uri).await?;
    
    let resp = client.request(req).await
        .map_err(|e| {
            warn!("Request to broker failed: {}", e.to_string());
            (StatusCode::BAD_GATEWAY, "Upstream error; see server logs.")
        })?;

    // Check reply's signature
    
    let (mut parts, body) = resp.into_parts();
    let mut bytes = body::to_bytes(body).await
        .map_err(|_| ERR_UPSTREAM)?;
    
    // TODO: Always return application/jwt from server.
    if ! bytes.is_empty() {
        let json = serde_json::from_slice::<Value>(&bytes);
        if json.is_err() {
            warn!("Answer is no valid JSON; returning as-is to client: \"{}\"", std::str::from_utf8(&bytes).unwrap_or_default());
        } else {
            let mut json = json.unwrap();
            if ! validate_and_remove_signatures(&mut json).await {
                return Err(ERR_VALIDATION);
            }
            bytes = serde_json::to_vec(&json).unwrap().into();
            debug!("Validated and stripped signature: \"{}\"", std::str::from_utf8(&bytes).unwrap_or("Unable to parse string as UTF-8"));
        }
    }
    
    let len = bytes.len();
    let body = Body::from(bytes);
    parts.headers.insert(header::CONTENT_LENGTH, len.into());
    let resp = Response::from_parts(parts, body);

    Ok(resp)
}

// TODO: This could be a middleware
async fn sign_request(req: Request<Body>, config: &config_proxy::Config, target_uri: &Uri) -> Result<Request<Body>, (StatusCode, &'static str)> {
    let (mut parts, body) = req.into_parts();
    let body = body::to_bytes(body).await
        .map_err(|_| ERR_BODY)?;
    let mut body = if body.is_empty() {
        debug!("Body is empty, substituting empty json.");
        serde_json::from_str::<Value>("{}").unwrap() // static input
    } else {
        match serde_json::from_slice::<Value>(&body) {
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
    let token_without_extended_signature = crypto_jwt::sign_to_jwt(&body).await
        .map_err(|e| {
            error!("Crypto failed: {}", e);
            ERR_INTERNALCRYPTO
        })?;
    let mut headers_mut = parts.headers;
    headers_mut.insert(header::DATE, HeaderValue::from_str(&fmt_http_date(SystemTime::now())).expect("Internal error: Unable to format system time"));
    let digest = crypto_jwt::make_extra_fields_digest(&parts.method, &parts.uri, &headers_mut)
        .map_err(|_| ERR_INTERNALCRYPTO)?;
    body.as_object_mut()
        .ok_or(ERR_BODY)?
        .insert("extra_fields_digest".to_string(), Value::String(digest));
    let token_with_extended_signature = crypto_jwt::sign_to_jwt(&body).await
        .map_err(|e| {
            error!("Crypto failed: {}", e);
            ERR_INTERNALCRYPTO
        })?;
    let length = token_without_extended_signature.len();
    let body: Body = token_without_extended_signature.into();
    let mut auth_header = String::from("SamplyJWT ");
    auth_header.push_str(&token_with_extended_signature);
    parts.uri = target_uri.clone();
    headers_mut.insert(header::HOST, config.broker_host_header.clone());
    headers_mut.insert(header::CONTENT_LENGTH, length.into());
    headers_mut.insert(header::CONTENT_TYPE, HeaderValue::from_str("application/jwt").unwrap()); // static input
    headers_mut.insert(header::AUTHORIZATION, HeaderValue::from_str(&auth_header)
        .map_err(|_| ERR_INTERNALCRYPTO)?);
    parts.headers = headers_mut;
    let req = Request::from_parts(parts, body);
    Ok(req)
}

#[async_recursion::async_recursion]
async fn validate_and_remove_signatures(json: &mut Value) -> bool {
    if json.is_array() {
        for inner in json.as_array_mut().unwrap() {
            if ! validate_and_remove_signatures(inner).await {
                return false;
            }
        }
    } else if json.is_object() {
        if  ! (validate_helper_value::<MsgTaskRequest>(json).await
            || validate_helper_value::<MsgTaskResult>(json).await
            || validate_helper_value::<MsgEmpty>(json).await) {
            return false;
        }
        let msg = json.as_object().unwrap().get("msg")
            .expect("Internal error: We just validated that this is a valid MsgSigned.")
            .to_owned();
        *json = msg;
        return true;
    }
    true
}

async fn validate_helper_value<M: Msg + DeserializeOwned + std::fmt::Debug> (value: &Value) -> bool {
    let value = value.clone();
    match serde_json::from_value::<MsgSigned<M>>(value) {
        Ok(msg) => {
            debug!("Verifying reply {:?}", msg);
            msg.verify().await.is_ok()
        },
        Err(_) => true, // Not of this type -> considered "valid"
    }
}
