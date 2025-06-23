
use std::fmt::Display;

use serde::{Deserialize, Serialize};

fn check_valid_id_part(id: &str) -> bool {
    id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !id.is_empty()
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
pub struct AppId(String);

impl AppId {
    pub fn new(id: impl Into<String>) -> Result<Self, BeamIdError> {
        let id: String = id.into();
        let mut iter = id.split('.');
        let Some(app_name) = iter.next() else {
            return Err(BeamIdError::InvalidNumberOfIdFragments);
        };
        if !check_valid_id_part(app_name) {
            return Err(BeamIdError::InvalidIdFragment);
        }
        let Some(proxy_name) = iter.next() else {
            return Err(BeamIdError::InvalidNumberOfIdFragments);
        };
        if !check_valid_id_part(proxy_name) {
            return Err(BeamIdError::InvalidIdFragment);
        }
        let Some(broker_id) = iter.next() else {
            return Err(BeamIdError::InvalidNumberOfIdFragments);
        };
        if broker_id.is_empty() {
            return Err(BeamIdError::InvalidIdFragment);
        }
        Ok(Self(id))
    }

    pub fn app_name(&self) -> &str {
        self.as_ref().split_once('.').expect("This is a valid app id").0
    }

    pub fn proxy_id(&self) -> ProxyId {
        ProxyId(self.proxy_id_str().to_owned())
    }

    pub fn proxy_id_str(&self) -> &str {
        self.as_ref().split_once('.').expect("This is a valid app id").1
    }

    /// Returns the AppId as a string slice without the broker part of the string
    /// ## Example
    /// app1.proxy1.broker => app1.proxy1
    pub fn hide_broker_name(&self) -> &str {
        let mut found_first_dot = false;
        self.as_ref().split_once(|c| match (c, found_first_dot) {
            ('.', true) => true,
            ('.', false) => {
                found_first_dot = true;
                false
            },
            _ => false,
        }).expect("This is a valid app id").0
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
pub struct ProxyId(String);

impl ProxyId {
    pub fn new(id: impl Into<String>) -> Result<Self, BeamIdError> {
        let id: String = id.into();
        let Some((proxy_name, broker_id)) = id.split_once('.') else {
            return Err(BeamIdError::InvalidNumberOfIdFragments);
        };
        if !check_valid_id_part(proxy_name) {
            return Err(BeamIdError::InvalidIdFragment);
        }
        if broker_id.is_empty() {
            return Err(BeamIdError::InvalidIdFragment);
        }
        Ok(Self(id))
    }

    /// Returns the proxies name without the broker id
    /// ## Example
    /// proxy1.broker => proxy1
    pub fn proxy_name(&self) -> &str {
        self.0
            .split_once('.')
            .map(|(proxy, _broker)| proxy)
            .expect("This is a valid proxy id")
    }

    pub fn hide_broker_name(&self) -> &str {
        self.0.split_once('.').expect("This is a valid proxy id").0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BeamIdError {
    InvalidNumberOfIdFragments,
    InvalidIdFragment,
}

impl std::error::Error for BeamIdError {}

impl Display for BeamIdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            BeamIdError::InvalidIdFragment => "Id fragment may only contain alphanumeric values.",
            BeamIdError::InvalidNumberOfIdFragments => "Id had an unexpected amount of fragments.",
        };
        f.write_str(text)
    }
}

macro_rules! impl_id {
    ($id:ident) => {
        impl $id {
            pub fn new_unchecked(id: impl Into<String>) -> Self {
                Self(id.into())
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

        impl<'de> Deserialize<'de> for $id {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer).map(Self::new)?.map_err(serde::de::Error::custom)
            }
        }
    };
}

impl_id!(AppId);
impl_id!(ProxyId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_id() {
        let app_id = AppId::new("app1.proxy1.broker").unwrap();
        assert_eq!(app_id.app_name(), "app1");
        assert_eq!(app_id.proxy_id().proxy_name(), "proxy1");
        assert_eq!(app_id.hide_broker_name(), "app1.proxy1");
    }

    #[test]
    fn test_invalid_app_id() {
        assert_eq!(AppId::new("app1..broker"), Err(BeamIdError::InvalidIdFragment));
        assert!(AppId::new("app1.proxy1.").is_err());
        assert!(AppId::new("app_2.proxy1").is_err());
    }
}