use std::{fmt::Display, sync::OnceLock};

use serde::{Deserialize, Serialize};

#[cfg(feature = "strict-ids")]
pub type AddressingId = crate::AppOrProxyId;

#[cfg(not(feature = "strict-ids"))]
pub type BeamIdType = crate::AppId;

#[cfg(feature = "strict-ids")]
static BROKER_ID: OnceLock<String> = OnceLock::new();

#[cfg(feature = "strict-ids")]
pub mod strict_ids {
    use super::*;



    #[derive(Debug, Clone, Serialize)]
    pub enum AppOrProxyId {
        App(AppId),
        Proxy(ProxyId),
    }

    impl AppOrProxyId {
        pub fn proxy_id(&self) -> ProxyId {
            match self {
                AppOrProxyId::App(app) => app.proxy_id(),
                AppOrProxyId::Proxy(proxy) => proxy.clone(),
            }
        }
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
}

#[derive(PartialEq, Debug)]
enum BeamIdType {
    AppId,
    ProxyId,
    BrokerId,
}

impl BeamIdType {
    const fn num_parts(&self) -> u8 {
        match self {
            BeamIdType::AppId => 3,
            BeamIdType::ProxyId => 2,
            BeamIdType::BrokerId => 1,
        }
    }
}

impl Display for BeamIdType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            BeamIdType::AppId => "AppId",
            BeamIdType::ProxyId => "ProxyId",
            BeamIdType::BrokerId => "BrokerId",
        };
        f.write_str(str)
    }
}

trait BeamId: Sized {
    #[cfg(feature = "strict-ids")]
    fn set_broker_id(id: String) {
        if let Err(value) = BROKER_ID.set(id) {
            assert_eq!(
                BROKER_ID.get().unwrap(),
                &value,
                "Tried to initialize broker_id with two different values"
            );
        }
    }

    #[cfg(feature = "strict-ids")]
    fn get_broker_id() -> &'static String {
        BROKER_ID.get().expect(
            "Global broker ID has not yet been set! This is required for feature strict-ids.",
        )
    }

    #[cfg(feature = "strict-ids")]
    fn strip_broker_id(id: &str) -> Result<&str, BeamIdError> {
        if let Some(rest) = id.strip_suffix(Self::get_broker_id()) {
            Ok(rest)
        } else {
            Err(BeamIdError::WrongBrokerId)
        }
    }

    fn new(id: &str) -> Result<Self, BeamIdError>;
}

fn get_id_type(id: &str) -> Result<BeamIdType, BeamIdError> {
    // One version with extra broker id checks
    todo!()
}

#[derive(Debug, Clone, Serialize)]
pub struct AppId(String);

impl AppId {
    pub fn app_name(&self) -> &str {
        let idx = self.0.find('.').unwrap_or(self.0.len() - 1);
        &self.0[0..idx]
    }

    pub fn proxy_id(&self) -> ProxyId {
        let proxy_id = self.0.get(self.app_name().len()..).expect("AppId should be valid");
        ProxyId::new(proxy_id).expect("This was a valid AppId so it should have a valid proxy part")
    }
}

impl BeamId for AppId {
    fn new(id: &str) -> Result<Self, BeamIdError> {
        match get_id_type(id)? {
            BeamIdType::AppId => Ok(Self(id.to_owned())),
            BeamIdType::ProxyId => todo!(),
            BeamIdType::BrokerId => todo!(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProxyId(String);

impl BeamId for ProxyId {
    fn new(id: &str) -> Result<Self, BeamIdError> {
        match get_id_type(id)? {
            BeamIdType::AppId => todo!(),
            BeamIdType::ProxyId => Ok(Self(id.to_owned())),
            BeamIdType::BrokerId => todo!(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BeamIdError {
    InvalidNumberOfIdFragments,
    InvalidIdFragment,
    #[cfg(feature = "strict-ids")]
    WrongBrokerId,
}

impl Display for BeamIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BeamIdError::InvalidIdFragment => todo!(),
            BeamIdError::InvalidNumberOfIdFragments => todo!(),
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

macro_rules! impl_deserialize {
    ($idType:ident) => {
        impl<'de> Deserialize<'de> for $idType {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                #[cfg(feature = "strict-ids")]
                return Self::new(<&str as Deserialize>::deserialize(deserializer)?)
                    .map_err(serde::de::Error::custom);
                #[cfg(not(feature = "strict-ids"))]
                return Self::new_unchecked(<String as Deserialize>::deserialize(deserializer)?);
            }
        }
    };
}

impl_deserialize!(AppId);
impl_deserialize!(ProxyId);
