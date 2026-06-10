use std::{
    convert::Infallible,
    str::FromStr,
    time::{Duration, SystemTime},
};

use axum::{
    body::Bytes, extract::{FromRef, Request, State}, http::{header, request::Parts, HeaderMap, HeaderValue, StatusCode, Uri}, response::{sse::Event, IntoResponse, Response, Sse}, routing::{any, get, put}, Json, RequestExt, Router
};
use futures::{
    stream::StreamExt,
    Stream, TryFutureExt,
};
use httpdate::fmt_http_date;
use rsa::{pkcs8::DecodePublicKey, RsaPublicKey};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use beam_lib::{AppId, AppOrProxyId, ProxyId};
use shared::{
    DecryptableMsg, EncryptableMsg, EncryptedMessage, EncryptedMsgTaskRequest, EncryptedMsgTaskResult, MessageType, Msg, MsgEmpty, MsgId, MsgSigned, MsgTaskRequest, MsgTaskResult, PlainMessage, crypto::{self, CryptoPublicPortion}, crypto_jwt, errors::SamplyBeamError, format_to_without_broker, http_client::SamplyHttpClient, reqwest, sse_event::SseEventType
};
use sse_stream::SseStream;
use tokio::{io::BufReader, task::id};
use tracing::{debug, error, info, trace, warn};

use crate::{auth::AuthenticatedApp, config::Config, PROXY_TIMEOUT};

#[derive(Clone, FromRef)]
pub(crate) struct TasksState {
    pub(crate) client: SamplyHttpClient,
    pub(crate) config: &'static Config,
}

pub(crate) fn router(client: &SamplyHttpClient, config: &'static Config) -> Router {
    let state = TasksState {
        client: client.clone(),
        config,
    };
    Router::new()
        // We need both path variants so the server won't send us into a redirect loop (/tasks, /tasks/, ...)
        .route("/v1/tasks", get(handler_task).post(handler_task))
        .route("/v1/tasks/{task_id}/results", get(handler_task))
        .route("/v1/tasks/{task_id}/results/{app_id}", put(handler_task))
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

pub(crate) async fn forward_request(
    mut req: Request<axum::body::Body>,
    config: &Config,
    sender: &AppId,
    client: &SamplyHttpClient,
) -> Result<reqwest::Response, Response> {
    // Create uri to contact broker
    let path = req.uri().path();
    let path_query = req
        .uri()
        .path_and_query()
        .map(|v| v.as_str())
        .unwrap_or(path);
    let target_uri =
        Uri::try_from(config.broker_uri.to_string() + path_query.trim_start_matches('/'))
            .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid path queried.").into_response())?;
    *req.uri_mut() = target_uri;

    req.headers_mut().append(
        header::VIA,
        HeaderValue::from_static(env!("SAMPLY_USER_AGENT")),
    );
    let (encrypted_msg, parts) = encrypt_request(req, &sender).await?;
    match &encrypted_msg {
        MessageType::MsgTaskRequest(task) => info!(from = %sender.hide_broker_name(), to = %format_to_without_broker(&task.to), id = %task.id, "Sending task"),
        MessageType::MsgTaskResult(result) => info!(from = %sender.hide_broker_name(), for = %result.task, "Submitting result"),
        #[cfg(feature = "sockets")]
        MessageType::MsgSocketRequest(socket_req) => info!(from = %socket_req.get_from().hide_broker(), to = %format_to_without_broker(&socket_req.get_to()), id = %socket_req.id, "Submitting socket request"),
        MessageType::MsgEmpty(..) => {},
    };
    let req = sign_request(encrypted_msg, parts, &config).await.map_err(IntoResponse::into_response)?;
    trace!("Requesting: {:?}", req);
    let resp = client.execute(req).await.map_err(|e| {
        if e.is_timeout() {
            debug!("Request to broker timed out after set proxy timeout of {PROXY_TIMEOUT}s");
            (StatusCode::GATEWAY_TIMEOUT, "Request to broker timed out ")
        } else {
            warn!("Request to broker failed: {}", e.to_string());
            (StatusCode::BAD_GATEWAY, "Upstream error; see server logs.")
        }.into_response()
    })?;
    if resp.status() == StatusCode::UNAUTHORIZED {
        error!("The Broker has rejected our request with 401 Unauthorized. This is likely because our beam certificate expired.");
        std::process::exit(401);
    }
    Ok(resp)
}

pub(crate) async fn handler_task(
    State(client): State<SamplyHttpClient>,
    State(config): State<&'static Config>,
    AuthenticatedApp(sender): AuthenticatedApp,
    headers: HeaderMap,
    req: Request,
) -> Response {
    let found = &headers
        .get(header::ACCEPT)
        .unwrap_or(&HeaderValue::from_static(""))
        .to_str()
        .unwrap_or_default()
        .split(',')
        .map(|part| part.trim())
        .find(|part| *part == "text/event-stream")
        .is_some();

    if *found {
        handler_tasks_stream(client, config, sender, req)
            .await
            .into_response()
    } else {
        handler_tasks_nostream(client, config, sender, req)
            .await
            .into_response()
    }
}

async fn handler_tasks_nostream(
    client: SamplyHttpClient,
    config: &Config,
    sender: AppId,
    req: Request,
) -> Result<Response, Response> {
    // Validate Query, forward to server, get response.

    let resp = forward_request(req, &config, &sender, &client).await?;
    let resp = axum::http::Response::from(resp);

    // Check reply's signature

    let (mut parts, body) = resp.into_parts();
    // Is this stupid? Yes. Is there an other way to do this? Yes by depending on hyper-body-util. Do you want to do that? No
    let mut bytes = reqwest::Response::from(axum::http::Response::new(body)).bytes().await.map_err(|e| {
        error!("Error receiving reply from the broker: {}", e);
        ERR_UPSTREAM.into_response()
    })?;

    // TODO: Always return application/jwt from server.
    if !bytes.is_empty() {
        if let Ok(json) = serde_json::from_slice::<Value>(&bytes) {
            let json = to_server_error(validate_and_decrypt(json, config).await)?;
            trace!("Decrypted Msg: {:#?}", json);
            bytes = serde_json::to_vec(&json).unwrap().into();
            trace!(
                "Validated and stripped signature: \"{}\"",
                std::str::from_utf8(&bytes).unwrap_or("Unable to parse string as UTF-8")
            );
            // Remove content length header as it has changed and is calculated by axum if unset
            parts.headers.remove(header::CONTENT_LENGTH);
        } else {
            warn!(
                "Answer is no valid JSON; returning as-is to client: \"{}\". Headers: {:?}",
                std::str::from_utf8(&bytes).unwrap_or("(unable to parse)"),
                parts
            );
        }
    }

    let body = axum::body::Body::from(bytes);

    Ok(Response::from_parts(parts, body))
}

async fn handler_tasks_stream(
    client: SamplyHttpClient,
    config: &'static Config,
    sender: AppId,
    req: Request,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, Response> {
    // Validate Query, forward to server, get response.

    let resp = forward_request(req, &config, &sender, &client).await?;
    
    let code = resp.status();
    if !code.is_success() {
        let error_msg = resp.text().await.unwrap_or("(unable to parse reply)".into());
        warn!("Got unexpected response code from server: {code}. Returning error message as-is: \"{error_msg}\"");
        return Err((code, error_msg).into_response());
    }

    let outgoing = async_stream::stream! {
        let mut reader = SseStream::from_byte_stream(resp.bytes_stream());

        while let Some(event) = reader.next().await {
            let event = match event {
                Ok(event) => event,
                Err(e) if sse_error_is_timeout(&e) => {
                    debug!("SSE connection timed out");
                    break;
                },
                Err(err) => {
                    error!("Got error reading SSE stream: {err}");
                    yield Ok(Event::default()
                        .event(SseEventType::Error)
                        .data("Error reading SSE stream from Broker (see Proxy logs for details)."));
                    continue;
                }
            };
            if event.retry.is_some() && event.event.is_none() && event.data.is_none() {
                error!("Got a retry message from the Broker, which is not yet supported.");
                continue;
            }
            // Check if this is a message or some control event
            let event_type = SseEventType::from_str(event.event.as_deref().unwrap_or("")).expect("Error in Infallible");
            let mut event_as_bytes = event.data.unwrap_or_default().into_bytes();
            let event_as_str = std::str::from_utf8(&event_as_bytes).unwrap_or("(unable to parse)");

            match &event_type {
                SseEventType::DeletedTask | SseEventType::WaitExpired => {
                    debug!("SSE: Got {event_type} message, forwarding to App.");
                    yield Ok(Event::default()
                        .event(event_type)
                        .data(event_as_str));
                    continue;
                },
                SseEventType::Error => {
                    warn!("SSE: The Broker has reported an error: {event_as_str}");
                    yield Ok(Event::default()
                        .event(event_type)
                        .data(event_as_str));
                    continue;
                },
                SseEventType::Undefined => {
                    error!("SSE: Got a message without event type -- discarding.");
                    continue;
                },
                SseEventType::Unknown(s) => {
                    error!("SSE: Got unknown event type: {s} -- discarding.");
                    continue;
                },
                SseEventType::NewResult => {
                    debug!("SSE: Got new result");
                }
                other => {
                    info!("Got \"{other}\" event -- parsing.");
                }
            }

            // Check reply's signature

            if !event_as_bytes.is_empty() {
                let Ok(json) = serde_json::from_slice::<Value>(&event_as_bytes) else {
                    warn!("Answer is no valid JSON; discarding: \"{event_as_str}\".");
                    // TODO: For some reason, compiler won't accept the following lines, so we can't inform the App about the problem.
                    //
                    // warn!("Answer is no valid JSON; returning as-is to client: \"{event_as_str}\".");
                    // yield Ok(Event::default()
                    //     .event(SseEventType::Error)
                    //     .data(format!("Broker sent invalid JSON: {event_as_str}")));
                    continue;
                };
                let json = match validate_and_decrypt(json, config).await {
                    Ok(json) => json,
                    Err(err) => {
                        warn!("Got an error decrypting Broker's reply: {err}");
                        continue;
                    }
                };
                trace!("Decrypted Msg: {:#?}",json);
                event_as_bytes = serde_json::to_vec(&json).unwrap();
                trace!(
                    "Validated and stripped signature: \"{}\"",
                    std::str::from_utf8(&event_as_bytes).unwrap_or("Unable to parse string as UTF-8")
                );
            }
            let as_string = std::str::from_utf8(&event_as_bytes).unwrap_or("(garbled_utf8)");
            let event = Event::default()
                .event(event_type)
                .data(as_string);
            yield Ok(event);
        }
    };
    // TODO: Somehow return correct error code (not always possible since headers are sent before long request)
    let sse = Sse::new(outgoing);
    Ok(sse)
}

pub(crate) fn sse_error_is_timeout(err: &sse_stream::Error) -> bool {
    match err {
        sse_stream::Error::Body(err) => err
            .downcast_ref::<reqwest::Error>()
            .is_some_and(reqwest::Error::is_timeout),
        _ => false,
    }
}

pub(crate) fn to_server_error<T>(res: Result<T, SamplyBeamError>) -> Result<T, Response> {
    res.map_err(|e| match e {
        SamplyBeamError::JsonParseError(e) => {
            warn!("{e}");
            ERR_UPSTREAM
        },
        SamplyBeamError::RequestValidationFailed(e) => {
            warn!("The answer was valid JSON but we were unable to validate and remove its signature. Err: {e}");
            ERR_VALIDATION
        },
        SamplyBeamError::SignEncryptError(_) => ERR_INTERNALCRYPTO,
        e => {
            warn!("Unhandled error {e}");
            (StatusCode::INTERNAL_SERVER_ERROR, "Unknown error")
        }
    }.into_response())
}

// TODO: This could be a middleware
pub async fn sign_request(
    body: EncryptedMessage,
    mut parts: Parts,
    config: &Config,
) -> Result<reqwest::Request, (StatusCode, &'static str)> {
    let from = body.get_from();

    let token_without_extended_signature = crypto_jwt::sign_to_jwt(&body, &config.crypto.privkey_rs256)
        .await
        .map_err(|e| {
            error!("Crypto failed: {}", e);
            ERR_INTERNALCRYPTO
        })?;
    let (_, sig) = token_without_extended_signature
        .rsplit_once('.')
        .ok_or_else(|| {
            error!(
                "Cannot get initial token's signature. Token: {}",
                token_without_extended_signature
            );
            ERR_INTERNALCRYPTO
        })?;
    let mut headers_mut = parts.headers;
    headers_mut.insert(
        header::DATE,
        HeaderValue::from_str(&fmt_http_date(SystemTime::now()))
            .expect("Internal error: Unable to format system time"),
    );
    let digest =
        crypto_jwt::make_extra_fields_digest(&parts.method, &parts.uri, &headers_mut, sig, &from)
            .map_err(|_| ERR_INTERNALCRYPTO)?;
    let token_with_extended_signature = crypto_jwt::sign_to_jwt(&digest, &config.crypto.privkey_rs256)
        .await
        .map_err(|e| {
            error!("Crypto failed: {}", e);
            ERR_INTERNALCRYPTO
        })?;
    let body: reqwest::Body = token_without_extended_signature.into();
    let mut auth_header = String::from("SamplyJWT ");
    auth_header.push_str(&token_with_extended_signature);
    headers_mut.insert(header::HOST, config.broker_host_header.clone());

    headers_mut.remove(header::CONTENT_LENGTH);
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
    Ok(req.try_into().expect("Uri to Url conversion should work"))
}

// This requires rustc 1.77
pub(crate) async fn validate_and_decrypt(json: Value, config: &Config) -> Result<Value, SamplyBeamError> {
    // It might be possible to use MsgSigned directly instead but there are issues impl Deserialize for MsgSigned<EncryptedMessage>
    #[derive(Deserialize)]
    struct MsgSignedHelper {
        jwt: String,
    }
    if let Value::Array(arr) = json {
        let mut results = Vec::with_capacity(arr.len());
        for value in arr {
            results.push(Box::pin(validate_and_decrypt(value, config)).await?);
        }
        Ok(Value::Array(results))
    } else if json.is_object() {
        match serde_json::from_value::<MsgSignedHelper>(json) {
            Ok(signed) => {
                let msg = MsgSigned::<EncryptedMessage>::verify(&signed.jwt)
                    .await?
                    .msg;
                match &msg {
                    MessageType::MsgTaskRequest(task) => info!(from = %task.get_from().hide_broker(), id = %task.id, "New task"),
                    MessageType::MsgTaskResult(result) => info!(from = %result.get_from().hide_broker(), for = %result.task, "New result"),
                    #[cfg(feature = "sockets")]
                    MessageType::MsgSocketRequest(socket_req) => info!(from = %socket_req.get_from().hide_broker(), id = %socket_req.id, "New socket request"),
                    MessageType::MsgEmpty(..) => {},
                };
                Ok(serde_json::to_value(decrypt_msg(msg, config)?).expect("Should serialize fine"))
            }
            Err(e) => Err(SamplyBeamError::JsonParseError(format!(
                "Failed to parse broker response as a signed encrypted message. Err is {e}"
            ))),
        }
    } else {
        Err(SamplyBeamError::JsonParseError(format!(
            "Broker respondend with invalid json {json:#?}"
        )))
    }
}

fn decrypt_msg<M: DecryptableMsg>(msg: M, config: &Config) -> Result<M::Output, SamplyBeamError> {
    msg.decrypt(
        &AppOrProxyId::Proxy(config.proxy_id.to_owned()),
        &config.crypto.privkey_rsa,
    )
}

async fn encrypt_request(
    mut req: Request,
    sender: &AppId,
) -> Result<(EncryptedMessage, Parts), Response> {
    let parts = req.extract_parts().await.unwrap();
    let body: bytes::Bytes = req.extract().await.map_err(|e| {
        warn!("Unable to read message body: {e}");
        ERR_BODY.into_response()
    })?;

    let msg = if body.is_empty() {
        debug!("Body is empty, substituting MsgEmpty.");
        PlainMessage::MsgEmpty(MsgEmpty {
            from: sender.clone().into(),
        })
    } else {
        match serde_json::from_slice(&body) {
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
                return Err(ERR_BODY.into_response());
            }
        }
    };
    // Sanity/security checks: From address sane?
    if msg.get_from() != sender {
        return Err(ERR_FAKED_FROM.into_response());
    }
    let body = encrypt_msg(msg).await.map_err(|e| {
        match e {
            SamplyBeamError::InvalidReceivers(proxies) => {
                (StatusCode::FAILED_DEPENDENCY, Json(proxies)).into_response()
            }
            e => {
                warn!("Encryption failed with: {e}");
                ERR_INTERNALCRYPTO.into_response()
            }
        }
    })?;
    Ok((body, parts))
}

async fn encrypt_msg<M: EncryptableMsg>(msg: M) -> Result<M::Output, SamplyBeamError> {
    let receivers_keys = crypto::get_proxy_public_keys(msg.get_to()).await?;
    msg.encrypt(&receivers_keys)
}
