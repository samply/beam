use std::collections::HashMap;

use axum::{async_trait, extract::{FromRequest, RequestParts}};
use hyper::{StatusCode, header::{HeaderName, self}};
use shared::ClientId;
use lazy_static::lazy_static;

use crate::config::{self, ApiKey};

lazy_static!{
    static ref CLIENTS: HashMap<ApiKey, ClientId> = {
        let (_, api_keys) = config::get_config()
            .expect("Unable to parse configuration");
        api_keys
    };
}

pub(crate) struct AuthenticatedProxyClient(pub(crate) ClientId);

#[async_trait]
impl<B: Send + Sync> FromRequest<B> for AuthenticatedProxyClient {
    type Rejection = (StatusCode, [(HeaderName, &'static str);1]);

    async fn from_request(req: &mut RequestParts<B>) -> Result<Self,Self::Rejection> {
        const SCHEME: &str = "ClientApiKey";
        const UNAUTHPROXY_ERR: (StatusCode,[(HeaderName, &'static str);1]) = (StatusCode::PROXY_AUTHENTICATION_REQUIRED, [(header::PROXY_AUTHENTICATE, SCHEME)]);
        const UNAUTH_ERR: (StatusCode,[(HeaderName, &'static str);1]) = (StatusCode::UNAUTHORIZED, [(header::PROXY_AUTHENTICATE, SCHEME)]);
        if let Some(auth) = req.headers().get(header::PROXY_AUTHORIZATION) {
            let auth = auth.to_str()
                .map_err(|_| UNAUTH_ERR)?;
            let mut auth = auth.split(' '); 
            if auth.next().unwrap_or("") != SCHEME {
                return Err(UNAUTH_ERR);
            }
            let api_key = auth.next().unwrap_or("");
            let client = CLIENTS.get(api_key);
            if client.is_none() {
                return Err(UNAUTH_ERR);
            }
            Ok(Self(client.unwrap().clone()))
        } else {
            Err(UNAUTHPROXY_ERR)
        }
    }
}