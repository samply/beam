use std::collections::HashMap;

use axum::{
    async_trait,
    extract::{FromRequest, FromRequestParts},
    http::request::Parts,
};
use hyper::{
    header::{self, HeaderName},
    Request, StatusCode,
};
use beam_lib::{AppId, AppOrProxyId};
use shared::{
    config, config_proxy
};

use tracing::{debug, Span, debug_span, warn};

pub(crate) struct AuthenticatedApp(pub(crate) AppId);

#[async_trait]
impl<S: Send + Sync> FromRequestParts<S> for AuthenticatedApp {
    type Rejection = (StatusCode, [(HeaderName, &'static str); 1]);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        const SCHEME: &str = "ApiKey";
        const UNAUTH_ERR: (StatusCode, [(HeaderName, &str); 1]) = (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, SCHEME)],
        );
        if let Some(auth) = parts.headers.get(header::AUTHORIZATION) {
            let auth_str = auth.to_str().map_err(|_| UNAUTH_ERR)?;
            let mut auth = auth_str.split(' ');
            if auth.next() != Some(SCHEME) {
                warn!(auth_str, "Invalid auth scheme");
                return Err(UNAUTH_ERR);
            }
            let Some(client_id) = auth.next().and_then(|s| AppId::new(s).ok()) else {
                warn!(auth_str, "Invalid app id");
                return Err(UNAUTH_ERR);
            };
            let Some(api_key_actual) = config::CONFIG_PROXY.api_keys.get(&client_id) else {
                warn!("App {client_id} not registered in proxy");
                return Err(UNAUTH_ERR);
            };
            let api_key_claimed = auth.next().ok_or(UNAUTH_ERR)?;
            if api_key_claimed != api_key_actual {
                warn!("App {client_id} provided the wrong api key");
                return Err(UNAUTH_ERR);
            }
            debug!("Request authenticated (ClientID {})", client_id);
            Span::current().record("from", AppOrProxyId::App(client_id.clone()).hide_broker());
            Ok(Self(client_id))
        } else {
            warn!("No auth header provided");
            Err(UNAUTH_ERR)
        }
    }
}
