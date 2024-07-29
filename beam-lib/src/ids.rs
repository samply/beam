
use std::fmt::Display;

use serde::{Deserialize, Serialize};

#[cfg(feature = "strict-ids")]
pub type AddressingId = crate::AppOrProxyId;

#[cfg(not(feature = "strict-ids"))]
pub type AddressingId = crate::AppId;

#[cfg(feature = "strict-ids")]
static BROKER_ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();

#[cfg(feature = "strict-ids")]
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum AppOrProxyId {
    App(AppId),
    Proxy(ProxyId),
}

#[cfg(feature = "strict-ids")]
impl PartialEq<AppId> for AppOrProxyId {
    fn eq(&self, other: &AppId) -> bool {
        match self {
            AppOrProxyId::App(app) => app == other,
            AppOrProxyId::Proxy(_) => false,
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
    pub fn new(id: &str) -> Result<Self, BeamIdError> {
        match get_id_type(id)? {
            BeamIdType::AppId => Ok(Self::App(AppId(id.to_owned()))),
            BeamIdType::ProxyId => Ok(Self::Proxy(ProxyId(id.to_owned()))),
            BeamIdType::BrokerId => Err(BeamIdError::InvalidIdKind),
        }
    }

    pub fn proxy_id(&self) -> ProxyId {
        match self {
            AppOrProxyId::App(app) => app.proxy_id(),
            AppOrProxyId::Proxy(proxy) => proxy.clone(),
        }
    }

    pub fn can_be_signed_by(&self, other: &impl AsRef<str>) -> bool {
        self.as_ref().ends_with(other.as_ref())
    }

    pub fn hide_broker(&self) -> &str {
        match self {
            AppOrProxyId::App(app) => app.hide_broker_name(),
            AppOrProxyId::Proxy(proxy) => proxy.proxy_name(),
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

#[cfg(feature = "strict-ids")]
fn strip_broker_id(id: &str) -> Result<&str, BeamIdError> {
    if let Some(rest) = id.strip_suffix(get_broker_id()) {
        Ok(rest)
    } else {
        Err(BeamIdError::WrongBrokerId)
    }
}

#[cfg(feature = "strict-ids")]
#[derive(PartialEq, Debug)]
pub(crate) enum BeamIdType {
    AppId,
    ProxyId,
    BrokerId,
}

macro_rules! impl_id {
    ($id:ident) => {
        impl $id {
            #[cfg(feature = "strict-ids")]
            pub fn new(id: impl Into<String>) -> Result<Self, BeamIdError> {
                {
                    let id = id.into();
                    if get_id_type(&id)? == BeamIdType::$id {
                        Ok(Self(id))
                    } else {
                        Err(BeamIdError::InvalidIdKind)
                    }
                }
            }
            
            pub fn new_unchecked(id: impl Into<String>) -> Self {
                Self(id.into())
            }

            #[cfg(feature = "strict-ids")]
            pub fn can_be_signed_by(&self, other: &impl AsRef<str>) -> bool {
                self.as_ref().ends_with(other.as_ref())
            }
        }

        impl AsRef<str> for $id {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl Display for $id {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

impl_id!(AppId);
impl_id!(ProxyId);

#[cfg(feature = "strict-ids")]
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
    if let Some(_) = split.next() {
        Err(BeamIdError::InvalidNumberOfIdFragments)
    } else {
        ret
    }
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
            .get(self.app_name().len() + 1..)
            .expect("AppId should be valid");
        ProxyId(proxy_id.to_string())
    }

    /// Returns the AppId as a string slice without the broker part of the string
    /// ## Example
    /// app1.proxy1.broker => app1.proxy1
    #[cfg(feature = "strict-ids")]
    pub fn hide_broker_name(&self) -> &str {
        let without_broker = strip_broker_id(&self.0).expect("Is valid id");
        &without_broker[..without_broker.len() - 1]
    }
}


#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
pub struct ProxyId(String);

impl ProxyId {

    /// Returns the proxies name without the broker id
    /// ## Example
    /// proxy1.broker => proxy1
    #[cfg(feature = "strict-ids")]
    pub fn proxy_name(&self) -> &str {
        self.0
            .split_once('.')
            .map(|(proxy, _broker)| proxy)
            .expect("This is a valid proxy id")
    }
}

#[cfg(feature = "strict-ids")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BeamIdError {
    InvalidNumberOfIdFragments,
    InvalidIdKind,
    InvalidIdFragment,
    #[cfg(feature = "strict-ids")]
    WrongBrokerId,
}

#[cfg(feature = "strict-ids")]
impl std::error::Error for BeamIdError {}

#[cfg(feature = "strict-ids")]
impl Display for BeamIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            BeamIdError::InvalidIdFragment => "Id fragment may only contain alphanumeric values.",
            BeamIdError::InvalidNumberOfIdFragments => "Id had an unexpected amount of fragments.",
            BeamIdError::InvalidIdKind => "Id parsed as a different kind of id then specified.",
            #[cfg(feature = "strict-ids")]
            BeamIdError::WrongBrokerId => {
                "The broker id part of the id did not match the global broker id."
            }
        };
        f.write_str(text)
    }
}

#[cfg(feature = "strict-ids")]
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
                #[cfg(feature = "strict-ids")]
                return Self::new(&String::deserialize(deserializer)?)
                    .map_err(serde::de::Error::custom);
                #[cfg(not(feature = "strict-ids"))]
                return Ok(Self::new_unchecked(String::deserialize(deserializer)?))
            }
        }
    };
}

impl_deserialize!(AppId);
impl_deserialize!(ProxyId);
#[cfg(feature = "strict-ids")]
impl_deserialize!(AppOrProxyId);

#[cfg(all(test, feature = "strict-ids"))]
mod tests {
    use super::*;

    #[test]
    fn test_str_has_type() {
        set_broker_id("broker.samply.de".to_string());
        assert_eq!(
            get_id_type("broker.samply.de").unwrap(),
            BeamIdType::BrokerId
        );
        assert_eq!(
            get_id_type("proxy23.broker.samply.de").unwrap(),
            BeamIdType::ProxyId
        );
        assert_eq!(
            get_id_type("app12.proxy23.broker.samply.de").unwrap(),
            BeamIdType::AppId
        );
        assert!(get_id_type("roker.samply.de").is_err());
        assert!(get_id_type("moreString.app12.proxy23.broker.samply.de").is_err());
    }

    #[test]
    fn test_appid_proxyid() {
        let app_id_str = "app.proxy1.broker.samply.de";

        set_broker_id("broker.samply.de".to_string());
        let app_id = AppId::new(app_id_str).unwrap();
        assert_eq!(
            app_id.proxy_id(),
            ProxyId::new("proxy1.broker.samply.de").unwrap()
        );
    }

    #[test]
    fn test_app_or_proxy_id() {
        let app_id_str = "app.proxy1.broker.samply.de";

        set_broker_id("broker.samply.de".to_string());
        let app_id = AppId::new(app_id_str).unwrap();
        let app_id_app: AppOrProxyId = app_id.clone().into();
        assert_eq!(app_id_app, app_id);

        let proxy_id = app_id.proxy_id();
        let app_id_proxy: AppOrProxyId = proxy_id.clone().into();
        assert_eq!(proxy_id, app_id_proxy.proxy_id());
    }
}
