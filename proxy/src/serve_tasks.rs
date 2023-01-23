use std::time::{Duration, SystemTime};

use axum::{Router, routing::any, response::Response, http::{HeaderValue, request::Parts}, extract::{State, FromRef}};
use httpdate::fmt_http_date;
use hyper::{
    body,
    client::{connect::Connect, HttpConnector},
    header, Body, Client, Request, StatusCode, Uri,
};
use hyper_proxy::ProxyConnector;
use hyper_tls::HttpsConnector;
use rsa::{pkcs8::DecodePublicKey, RsaPublicKey};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use shared::{
    beam_id::{AppId, AppOrProxyId, ProxyId}, config::{self, CONFIG_PROXY}, config_proxy, crypto_jwt, errors::SamplyBeamError, EncMsg, DecMsg,
    EncryptedMsgTaskRequest, EncryptedMsgTaskResult, Msg, MsgEmpty, MsgId, MsgSigned,
    MsgTaskRequest, MsgTaskResult, crypto, http_proxy::SamplyHttpClient,
};
use tracing::{debug, error, warn};

use crate::auth::AuthenticatedApp;

#[derive(Clone, FromRef)]
struct TasksState {
    client: SamplyHttpClient,
    config: config_proxy::Config
}

pub(crate) fn router(client: &SamplyHttpClient) -> Router {
    let config = config::CONFIG_PROXY.clone();
    let state = TasksState {
        client: client.clone(),
        config,
    };
    Router::new()
        // We need both path variants so the server won't send us into a redirect loop (/tasks, /tasks/, ...)
        .route("/v1/tasks", any(handler_tasks))
        .route("/v1/tasks/*path", any(handler_tasks))
        .with_state(state)
}

const ERR_BODY: (StatusCode, &str) = (StatusCode::BAD_REQUEST, "Invalid body");
const ERR_INTERNALCRYPTO: (StatusCode, &str) = (
    StatusCode::INTERNAL_SERVER_ERROR,
    "Cryptography failed; see server logs.",
);
const ERR_UPSTREAM: (StatusCode, &str) =
    (StatusCode::BAD_GATEWAY, "Unable to parse server's reply.");
const ERR_VALIDATION: (StatusCode, &str) = (
    StatusCode::BAD_GATEWAY,
    "Unable to verify signature in server reply.",
);
const ERR_FAKED_FROM: (StatusCode, &str) = (
    StatusCode::UNAUTHORIZED,
    "You are not authorized to send on behalf of this app.",
);

async fn handler_tasks(
    State(client): State<SamplyHttpClient>,
    State(config): State<config_proxy::Config>,
    AuthenticatedApp(sender): AuthenticatedApp,
    req: Request<Body>,
) -> Result<Response<Body>, (StatusCode, &'static str)> {
    let path = req.uri().path();
    let path_query = req
        .uri()
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(path);

    let target_uri =
        Uri::try_from(config.broker_uri.to_string() + path_query.trim_start_matches('/'))
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid path queried."))?;
    let (body, parts) = encrypt_request(req, &sender).await?;
    let req = sign_request(body, parts, &config, &target_uri).await?;

    let resp = client.request(req).await.map_err(|e| {
        warn!("Request to broker failed: {}", e.to_string());
        (StatusCode::BAD_GATEWAY, "Upstream error; see server logs.")
    })?;

    // Check reply's signature

    let (mut parts, body) = resp.into_parts();
    let mut bytes = body::to_bytes(body).await.map_err(|e| {
        error!("Error receiving reply from the broker: {}", e);
        ERR_UPSTREAM
    })?;

    // TODO: Always return application/jwt from server.
    if !bytes.is_empty() {
        let json = serde_json::from_slice::<Value>(&bytes);
        if json.is_err() {
            warn!(
                "Answer is no valid JSON; returning as-is to client: \"{}\"",
                std::str::from_utf8(&bytes).unwrap_or_default()
            );
        } else {
            let mut json = json.unwrap();
            if !validate_and_remove_signatures(&mut json).await {
                warn!("The answer was valid JSON but we were unable to validate and remove its signature. The osffending JSON was: {}", json);
                return Err(ERR_VALIDATION);
            }
            decryption_helper(&mut json).or( Err(ERR_INTERNALCRYPTO))?;
            debug!("Decrypted Msg: {:#?}",json);
            bytes = serde_json::to_vec(&json).unwrap().into();
            debug!(
                "Validated and stripped signature: \"{}\"",
                std::str::from_utf8(&bytes).unwrap_or("Unable to parse string as UTF-8")
            );
        }
    }

    let len = bytes.len();
    let body = Body::from(bytes);
    parts.headers.insert(header::CONTENT_LENGTH, len.into());
    parts.headers.insert(hyper::header::USER_AGENT, HeaderValue::from_static(env!("SAMPLY_USER_AGENT")));
    let resp = Response::from_parts(parts, body);

    Ok(resp)
}

// TODO: This could be a middleware
async fn sign_request(
    mut body: Value,
    mut parts: Parts,
    config: &config_proxy::Config,
    target_uri: &Uri,
) -> Result<Request<Body>, (StatusCode, &'static str)> {

    let token_without_extended_signature = crypto_jwt::sign_to_jwt(&body).await.map_err(|e| {
        error!("Crypto failed: {}", e);
        ERR_INTERNALCRYPTO
    })?;
    let mut headers_mut = parts.headers;
    headers_mut.insert(
        header::DATE,
        HeaderValue::from_str(&fmt_http_date(SystemTime::now()))
            .expect("Internal error: Unable to format system time"),
    );
    let digest = crypto_jwt::make_extra_fields_digest(&parts.method, &parts.uri, &headers_mut)
        .map_err(|_| ERR_INTERNALCRYPTO)?;
    body.as_object_mut()
        .ok_or_else(|| {
            warn!("Unable to read body as JSON map");
            ERR_BODY
        })?
        .insert("extra_fields_digest".to_string(), Value::String(digest));
    let token_with_extended_signature = crypto_jwt::sign_to_jwt(&body).await.map_err(|e| {
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
    headers_mut.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str("application/jwt").unwrap(),
    ); // static input
    headers_mut.insert(
        header::AUTHORIZATION,
        HeaderValue::from_str(&auth_header).map_err(|_| ERR_INTERNALCRYPTO)?,
    );
    parts.headers = headers_mut;
    let req = Request::from_parts(parts, body);
    Ok(req)
}

#[async_recursion::async_recursion]
async fn validate_and_remove_signatures(json: &mut Value) -> bool {
    if json.is_array() {
        for inner in json.as_array_mut().unwrap() {
            if !validate_and_remove_signatures(inner).await {
                return false;
            }
        }
    } else if json.is_object() {
        if !(validate_helper_value::<MsgTaskRequest>(json).await
            || validate_helper_value::<MsgTaskResult>(json).await
            || validate_helper_value::<MsgEmpty>(json).await)
        {
            return false;
        }
        let msg = json
            .as_object()
            .unwrap()
            .get("msg")
            .expect("Internal error: We just validated that this is a valid MsgSigned.")
            .to_owned();
        *json = msg;
        return true;
    }
    true
}

async fn validate_helper_value<M: Msg + DeserializeOwned + std::fmt::Debug>(value: &Value) -> bool {
    let value = value.clone();
    match serde_json::from_value::<MsgSigned<M>>(value) {
        Ok(msg) => {
            debug!("Verifying reply {:?}", msg);
            msg.verify().await.is_ok()
        }
        Err(_) => true, // Not of this type -> considered "valid"
    }
}

fn decryption_helper(value: &mut Value) -> Result<(), SamplyBeamError> {
    if value.is_array() {
        for inner in value.as_array_mut().unwrap() {
            decryption_helper(inner)?;
        }
    } else if value.is_object() {
        if is_message_type::<EncryptedMsgTaskRequest>(value) {
            *value = decrypt_msg::<MsgTaskRequest, EncryptedMsgTaskRequest>(value)?;
            return Ok(());
        } else if is_message_type::<EncryptedMsgTaskResult>(value) {
            *value = decrypt_msg::<MsgTaskResult, EncryptedMsgTaskResult>(value)?;
            return Ok(());
        }
    }
    *value = value.clone();
    Ok(())
}

// Once specialization becomes stable, implement in Msg trait (see https://stackoverflow.com/questions/60138397/how-to-test-for-type-equality-in-rust)
fn is_message_type<M: Msg + DeserializeOwned + std::fmt::Debug>(value: &Value) -> bool {
    let value = value.clone();
    match serde_json::from_value::<M>(value) {
        Ok(_msg) => true,
        Err(_) => false, // Not of this type -> considered "valid"
    }
}

fn decrypt_msg<T: Msg + DeserializeOwned + Serialize, M: EncMsg<T> + DeserializeOwned + Serialize + std::fmt::Debug>(
    value: &Value,
) -> Result<Value, SamplyBeamError> {
    let enc_value = value.clone();
        match serde_json::from_value::<M>(enc_value) {
        Ok(msg) => serde_json::to_value(msg.decrypt(&AppOrProxyId::ProxyId(CONFIG_PROXY.proxy_id.to_owned()), crypto::get_own_privkey())?).map_err(|e| {
            SamplyBeamError::SignEncryptError(format!("Cannot decrypt message: {}", e).into())
        }),
        Err(e) => Err(SamplyBeamError::SignEncryptError(format!("Error decrypting message: {}",e))),
    }
}

async fn encrypt_request(
    req: Request<Body>,
    sender: &AppId,
) -> Result<(Value, Parts), (StatusCode, &'static str)> {
    let (parts, body) = req.into_parts();
    let body = body::to_bytes(body).await.map_err(|e| {
        warn!("Unable to read message body: {e}");
        ERR_BODY
    })?;

    let body = if body.is_empty() {
        debug!("Body is empty, substituting MsgEmpty.");
        let empty = MsgEmpty {
            from: sender.into(),
        };
        serde_json::to_value(empty).unwrap()
    } else {
        match serde_json::from_slice::<Value>(&body) {
            Ok(val) => {
                debug!("Body is valid json");
                val
            }
            Err(e) => {
                warn!(
                    "Received Body is invalid json: {}. Body was {}",
                    e,
                    std::str::from_utf8(&body).unwrap_or("(not valid UTF-8)")
                );
                return Err(ERR_BODY);
            }
        }
    };
    // Sanity/security checks: From address sane?
    let msg = serde_json::from_value::<MsgEmpty>(body.clone()).map_err(|e| {
        warn!("Received body did not deserialize into MsgEmpty: {e}");
        ERR_BODY
    })?;
    if msg.get_from() != sender {
        return Err(ERR_FAKED_FROM);
    }
    // What Message is sent?
    if is_message_type::<MsgTaskRequest>(&body){
        let body = encrypt_msg::<EncryptedMsgTaskRequest, MsgTaskRequest>(&body).await.map_err(|_| ERR_INTERNALCRYPTO)?;
        Ok((body, parts))
    }
    else if is_message_type::<MsgTaskResult>(&body){
        let body = encrypt_msg::<EncryptedMsgTaskResult, MsgTaskResult>(&body).await.map_err(|_| ERR_INTERNALCRYPTO)?;
        Ok((body, parts))
    } else {
        Ok((body, parts))
    }
}

async fn encrypt_msg<T: Msg + DeserializeOwned, M: DecMsg<T> + DeserializeOwned + Serialize + std::fmt::Debug>(
    value: &Value,
) -> Result<Value, SamplyBeamError> {
    let value = value.clone();
    match serde_json::from_value::<M>(value) {
        Ok(msg) => {
            let receivers_keys = crypto::get_proxy_public_keys(msg.get_to()).await?;
            serde_json::to_value(msg.encrypt(&receivers_keys)?).map_err(|e| {
                SamplyBeamError::SignEncryptError(format!("Cannot decrypt message: {}", e).into())
            })
        },
        Err(e) => Err(SamplyBeamError::SignEncryptError(format!("Cannot decrypt message: {}", e).into())),
    }
}
