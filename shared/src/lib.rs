#![allow(unused_imports)]

use dataobjects::beam_id::{BeamId, AppId, AppOrProxyId};
use crypto_jwt::extract_jwt;
use errors::SamplyBeamError;
use serde_json::{Value, json};
use static_init::dynamic;
use tracing::debug;

use std::{time::Duration, ops::Deref, fmt::Display};

use rand::Rng;
use serde::{Deserialize, Serialize, de::Visitor};
use std::{collections::HashMap, str::FromStr};
use uuid::Uuid;
use dataobjects::{Msg, MsgSigned};

mod traits;
pub mod logger;
pub mod crypto;
pub mod crypto_jwt;
pub mod errors;

pub mod config;
pub mod config_shared;
// #[cfg(feature = "config-for-broker")]
pub mod config_broker;
// #[cfg(feature = "config-for-proxy")]
pub mod config_proxy;

pub mod middleware;
pub mod http_proxy;

pub mod examples;


impl<M: Msg> MsgSigned<M> {
    pub async fn verify(&self) -> Result<(), SamplyBeamError> {
        // Signature valid?
        let (proxy_public_info, _, content) 
            = extract_jwt(&self.sig).await?;

        // Message content matches token?
        let val = serde_json::to_value(&self.msg)
        .expect("Internal error: Unable to interpret already parsed message to JSON Value.");
        if content.custom != val {
            return Err(SamplyBeamError::RequestValidationFailed);
        }

        // From field matches CN in certificate?
        if ! self.get_from().can_be_signed_by(&proxy_public_info.beam_id) {
            return Err(SamplyBeamError::RequestValidationFailed);
        }
        debug!("Message has been verified succesfully.");
        Ok(())
    }
}


