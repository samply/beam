use std::collections::HashMap;

use axum::{async_trait, extract::{FromRequest, RequestParts}};
use hyper::{StatusCode, header::{HeaderName, self}};
use shared::{config_proxy, config, beam_id::{AppId, BeamId}};

use tracing::debug;

pub(crate) struct AuthenticatedApp(pub(crate) AppId);

#[async_trait]
impl<B: Send + Sync> FromRequest<B> for AuthenticatedApp {
    type Rejection = (StatusCode, [(HeaderName, &'static str);1]);

    async fn from_request(req: &mut RequestParts<B>) -> Result<Self,Self::Rejection> {
        const SCHEME: &str = "ApiKey";
        const UNAUTH_ERR: (StatusCode,[(HeaderName, &str);1]) = (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, SCHEME)]);
        if let Some(auth) = req.headers().get(header::AUTHORIZATION) {
            let auth = auth.to_str()
                .map_err(|_| UNAUTH_ERR)?;
            let mut auth = auth.split(' '); 
            if auth.next().unwrap_or("") != SCHEME {
                return Err(UNAUTH_ERR);
            }
            let client_id = auth.next().unwrap_or("");
            let client_id = AppId::new(client_id).map_err(|_| UNAUTH_ERR)?;
            let api_key_actual = 
                config::CONFIG_PROXY.api_keys.get(&client_id)
                .ok_or(UNAUTH_ERR)?;
            let api_key_claimed = 
                auth.next()
                .ok_or(UNAUTH_ERR)?;
            if api_key_claimed != api_key_actual {
                return Err(UNAUTH_ERR);
            }
            debug!("Request authenticated (ClientID {})", client_id);
            Ok(Self(client_id))
        } else {
            Err(UNAUTH_ERR)
        }
    }
}