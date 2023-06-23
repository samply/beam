use std::{fmt::Display, sync::OnceLock};

use serde::{Serialize, Deserialize};

#[cfg(feature = "strict-ids")]
static BROKER_ID: OnceLock<String> = OnceLock::new();

trait BeamId: Sized {
    #[cfg(feature = "strict-ids")]
    fn set_broker_id(id: String) {
        if let Err(value) = BROKER_ID.set(id) {
            assert_eq!(
                BROKER_ID.get().unwrap(), &value,
                "Tried to initialize broker_id with two different values"
            );
        }
    }

    #[cfg(feature = "strict-ids")]
    fn get_broker_id() -> &'static String {
        BROKER_ID.get().expect("Global broker ID has not yet been set! This is required for feature strict-ids.")
    }

    fn id_check(id: &str) -> Result<(), BeamIdError> {
        check_valid_id_chars(id)?;
        #[cfg(feature = "strict-ids")]
        if !id.ends_with(Self::get_broker_id()) {
            return Err(BeamIdError::WrongBrokerId);
        }
        Ok(())
    }
    fn new(id: &str) -> Result<Self, BeamIdError> {
        Self::id_check(id)?;
        Self::check_parts(id)
    }
    
    fn check_parts(id: &str) -> Result<Self, BeamIdError>;
}

#[derive(Debug, Clone, Serialize)]
pub struct AppId(String);

impl<'de> Deserialize<'de> for AppId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(<&str as Deserialize>::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl BeamId for AppId {
    fn check_parts(id: &str) -> Result<Self, BeamIdError> {
        if id.splitn(3, '.').count() == 3 {
            Ok(Self(id.to_owned()))
        } else {
            Err(BeamIdError::NotAnAppId)
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProxyId(String);

impl<'de> Deserialize<'de> for ProxyId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::check_parts(<&str as Deserialize>::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl BeamId for ProxyId {
    fn check_parts(id: &str) -> Result<Self, BeamIdError> {
        if id.splitn(3, '.').count() >= 2 {
            Ok(Self(id.to_owned()))
        } else {
            Err(BeamIdError::NotAProxyId)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AppOrProxyId {
    App(AppId),
    Proxy(ProxyId)
}

impl From<AppId> for AppOrProxyId {
    fn from(app: AppId) -> Self {
        AppOrProxyId::App(app)
    }
}

impl From<ProxyId> for AppOrProxyId {
    fn from(proxy: ProxyId) -> Self {
        AppOrProxyId::Proxy(proxy)
    }
}

#[cfg(feature = "strict-ids")]
impl BeamId for AppOrProxyId {
    fn check_parts(id: &str) -> Result<Self, BeamIdError> {
        todo!("Rethink strict parsing")
    }
}


#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BeamIdError {
    NotAProxyId,
    NotAnAppId,
    InvalidIdFragment,
    #[cfg(feature = "strict-ids")]
    WrongBrokerId,
}

impl Display for BeamIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BeamIdError::NotAProxyId => todo!(),
            BeamIdError::NotAnAppId => todo!(),
            BeamIdError::InvalidIdFragment => todo!(),
            #[cfg(feature = "strict-ids")]
            BeamIdError::WrongBrokerId => todo!(),
        }
    }
}

fn check_valid_id_chars(id: &str) -> Result<(), BeamIdError> {
    for char in id.chars() {
        if !(char.is_alphanumeric() || char == '-') {
            return Err(BeamIdError::InvalidIdFragment);
        }
    }
    Ok(())
}
