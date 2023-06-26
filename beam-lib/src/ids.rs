use std::{fmt::Display, sync::OnceLock, error::Error};

use serde::{Deserialize, Serialize};

#[cfg(feature = "strict-ids")]
pub type AddressingId = crate::AppOrProxyId;

#[cfg(not(feature = "strict-ids"))]
pub type AddressingId = crate::AppId;

#[cfg(feature = "strict-ids")]
static BROKER_ID: OnceLock<String> = OnceLock::new();

#[cfg(feature = "strict-ids")]
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
pub enum AppOrProxyId {
    App(AppId),
    Proxy(ProxyId),
}

#[cfg(feature = "strict-ids")]
impl BeamId for AppOrProxyId {
    fn new(id: &str) -> Result<Self, BeamIdError> {
        match get_id_type(id)? {
            BeamIdType::AppId => Ok(Self::App(AppId(id.to_owned()))),
            BeamIdType::ProxyId => Ok(Self::Proxy(ProxyId(id.to_owned()))),
            BeamIdType::BrokerId => Err(BeamIdError::InvalidIdKind),
        }
    }
}

#[cfg(feature = "strict-ids")]
impl AsRef<str> for AppOrProxyId {
    fn as_ref(&self) -> &str {
        match self {
            AppOrProxyId::App(a) => a.as_ref(),
            AppOrProxyId::Proxy(p) => p.as_ref(),
        }
    }
}

#[cfg(feature = "strict-ids")]
impl Display for AppOrProxyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            AppOrProxyId::App(app) => &app.0,
            AppOrProxyId::Proxy(proxy) => &proxy.0,
        })
    }
}

#[cfg(feature = "strict-ids")]
impl AppOrProxyId {
    pub fn proxy_id(&self) -> ProxyId {
        match self {
            AppOrProxyId::App(app) => app.proxy_id(),
            AppOrProxyId::Proxy(proxy) => proxy.clone(),
        }
    }

    pub fn hide_broker(&self) -> String {
        match self {
            AppOrProxyId::App(app) => {
                let without_broker = strip_broker_id(&app.0).expect("Is valid id");
                without_broker[..without_broker.len() - 1].to_owned()
            }
            AppOrProxyId::Proxy(proxy) => proxy
                .0
                .split_once('.')
                .map(|(proxy, _broker)| proxy)
                .unwrap_or_default()
                .to_string(),
        }
    }
}

#[cfg(feature = "strict-ids")]
impl From<AppId> for AppOrProxyId {
    fn from(app: AppId) -> Self {
        AppOrProxyId::App(app)
    }
}

#[cfg(feature = "strict-ids")]
impl From<ProxyId> for AppOrProxyId {
    fn from(proxy: ProxyId) -> Self {
        AppOrProxyId::Proxy(proxy)
    }
}

#[derive(PartialEq, Debug)]
enum BeamIdType {
    AppId,
    ProxyId,
    BrokerId,
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

#[cfg(feature = "strict-ids")]
pub fn set_broker_id(id: String) {
    if let Err(value) = BROKER_ID.set(id) {
        assert_eq!(
            BROKER_ID.get().unwrap(),
            &value,
            "Tried to initialize broker_id with two different values"
        );
    }
}

#[cfg(feature = "strict-ids")]
pub fn get_broker_id() -> &'static String {
    BROKER_ID
        .get()
        .expect("Global broker ID has not yet been set! This is required for feature strict-ids.")
}

fn strip_broker_id(id: &str) -> Result<&str, BeamIdError> {
    #[cfg(feature = "strict-ids")]
    if let Some(rest) = id.strip_suffix(get_broker_id()) {
        Ok(rest)
    } else {
        Err(BeamIdError::WrongBrokerId)
    }
    #[cfg(not(feature = "strict-ids"))]
    {
        let Some(i) = id.rfind('.') else {
            return Ok("");
        };
        Ok(&id[i..id.len() - 1])
    }
}

pub trait BeamId: Sized + AsRef<str> {
    fn new(id: &str) -> Result<Self, BeamIdError>;

    #[cfg(feature = "strict-ids")]
    fn can_be_signed_by(&self, other: &impl AsRef<str>) -> bool {
        self.as_ref().ends_with(other.as_ref())
    }
}

fn get_id_type(id: &str) -> Result<BeamIdType, BeamIdError> {
    let rest = strip_broker_id(id)?;
    let Some(rest) = rest.strip_suffix('.') else {
        return Ok(BeamIdType::BrokerId);
    };
    let mut split = rest.split('.');
    let ret = match (split.next(), split.next()) {
        (Some(proxy), None) => {
            check_valid_id_part(proxy)?;
            Ok(BeamIdType::ProxyId)
        }
        (Some(app), Some(proxy)) => {
            check_valid_id_part(app)?;
            check_valid_id_part(proxy)?;
            Ok(BeamIdType::AppId)
        }
        (None, _) => unreachable!(),
    };
    #[cfg(feature = "strict-ids")]
    if let Some(_) = split.next() {
        Err(BeamIdError::InvalidNumberOfIdFragments)
    } else {
        ret
    }
    #[cfg(not(feature = "strict-ids"))]
    ret
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
pub struct AppId(String);

impl AppId {
    pub fn app_name(&self) -> &str {
        let idx = self.0.find('.').unwrap_or(self.0.len() - 1);
        &self.0[0..idx]
    }

    pub fn proxy_id(&self) -> ProxyId {
        let proxy_id = self
            .0
            .get(self.app_name().len()..)
            .expect("AppId should be valid");
        ProxyId::new(proxy_id).expect("This was a valid AppId so it should have a valid proxy part")
    }
}

impl AsRef<str> for AppId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl BeamId for AppId {
    fn new(id: &str) -> Result<Self, BeamIdError> {
        match get_id_type(id)? {
            BeamIdType::AppId => Ok(Self(id.to_owned())),
            _ => Err(BeamIdError::InvalidIdKind),
        }
    }
}

impl Display for AppId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
pub struct ProxyId(String);

impl AsRef<str> for ProxyId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl BeamId for ProxyId {
    fn new(id: &str) -> Result<Self, BeamIdError> {
        match get_id_type(id)? {
            BeamIdType::ProxyId => Ok(Self(id.to_owned())),
            _ => Err(BeamIdError::InvalidIdKind),
        }
    }
}

impl Display for ProxyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BeamIdError {
    InvalidNumberOfIdFragments,
    InvalidIdKind,
    InvalidIdFragment,
    #[cfg(feature = "strict-ids")]
    WrongBrokerId,
}

impl Error for BeamIdError {}

impl Display for BeamIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            BeamIdError::InvalidIdFragment => "Id fragment may only countain alphanumeric values.",
            BeamIdError::InvalidNumberOfIdFragments => "Id had an unexpected amout of fragments.",
            BeamIdError::InvalidIdKind => "Id parsed as a diffrent kind of id then specified.",
            #[cfg(feature = "strict-ids")]
            BeamIdError::WrongBrokerId => {
                "The broker id part of the id did not match the global broker id."
            }
        };
        f.write_str(text)
    }
}

fn check_valid_id_part(id: &str) -> Result<(), BeamIdError> {
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
                return Self::new(<&str as Deserialize>::deserialize(deserializer)?)
                    .map_err(serde::de::Error::custom);
            }
        }
    };
}

impl_deserialize!(AppId);
impl_deserialize!(ProxyId);
#[cfg(feature = "strict-ids")]
impl_deserialize!(AppOrProxyId);
