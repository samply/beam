use std::{ops::Deref, fmt::Display, hash::Hash};

use serde::{Serialize, Deserialize};

use crate::{errors::SamplyBrokerError, config};

#[derive(PartialEq)]
pub enum BeamIdType {
    AppId,
    ProxyId,
    BrokerId
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

pub trait BeamId: Display + Sized + PartialEq + Eq + Hash {
    fn str_has_type(value: &str) -> Result<BeamIdType,SamplyBrokerError> {
        let domain = &config::CONFIG_SHARED.broker_domain;
        let mut split = value.split('.').rev();
        // Broker
        let part = split.next();
        if part.is_none() || part.unwrap() != domain {
            return Err(SamplyBrokerError::InvalidBeamId(format!("Beam ID must end with {domain}")));
        }
        let part = split.next();
        if part.is_none() {
            return Ok(BeamIdType::BrokerId);
        }
        check_valid_id_part(part.unwrap())?;
        let part = split.next();
        if part.is_none() {
            return Ok(BeamIdType::ProxyId);
        }
        check_valid_id_part(part.unwrap())?;
        if let Some(s) = split.next() {
            return Err(SamplyBrokerError::InvalidBeamId(format!("Beam ID must not continue left of AppID part: {s}")));
        }
        Ok(BeamIdType::AppId)
    }
    fn has_type(&self) -> BeamIdType { // This is for &self, so we can assume the existing ID is correct
        Self::str_has_type(&self.value()).unwrap()
    }
    fn value(&self) -> &String;
    fn new(id: &str) -> Result<Self,SamplyBrokerError>;
    fn can_be_signed_by<B: BeamId>(&self, other_id: &B) -> bool {
        return self.value().ends_with(other_id.value());
    }
}

fn check_valid_id_part(id: &str) -> Result<(),SamplyBrokerError> {
    for char in id.chars() {
        if !(char.is_alphanumeric() || char == '-'){
            return Err(SamplyBrokerError::InvalidBeamId(format!("Invalid Beam ID element: {id}")));
        }
    }
    Ok(())
}

impl Display for AppId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.value())
    }
}

impl Display for ProxyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.value())
    }
}

impl Display for BrokerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.value())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct AppId(String);
impl BeamId for AppId {
    fn value(&self) -> &String {
        &self.0
    }

    fn new(id: &str) -> Result<Self,SamplyBrokerError> {
        let given_type = Self::str_has_type(id)?;
        if given_type != BeamIdType::AppId {
            return Err(SamplyBrokerError::InvalidBeamId(format!("{id} is a {given_type}, not an AppId.")));
        }
        Ok(Self(id.to_string()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct ProxyId(String);
impl BeamId for ProxyId {
    fn value(&self) -> &String {
        &self.0
    }

    fn new(id: &str) -> Result<Self,SamplyBrokerError> {
        let given_type = Self::str_has_type(id)?;
        if given_type != BeamIdType::ProxyId {
            return Err(SamplyBrokerError::InvalidBeamId(format!("{id} is a {given_type}, not a ProxyId.")));
        }
        Ok(Self(id.to_string()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BrokerId(String);
impl BeamId for BrokerId {
    fn value(&self) -> &String {
        &self.0
    }

    fn new(id: &str) -> Result<Self,SamplyBrokerError> {
        let given_type = Self::str_has_type(id)?;
        if given_type != BeamIdType::BrokerId {
            return Err(SamplyBrokerError::InvalidBeamId(format!("{id} is a {given_type}, not a BrokerId.")));
        }
        Ok(Self(id.to_string()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum AnyBeamId {
    AppId(AppId),
    ProxyId(ProxyId),
    BrokerId(BrokerId),
}

impl Display for AnyBeamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.value())
    }
}

impl BeamId for AnyBeamId {
    fn value(&self) -> &String {
        match self {
            AnyBeamId::AppId(e) => e.value(),
            AnyBeamId::ProxyId(e) => e.value(),
            AnyBeamId::BrokerId(e) => e.value(),
        }
    }

    fn new(id: &str) -> Result<Self,SamplyBrokerError> {
        let res = match AppId::str_has_type(id)? { // TODO: Better use trait
            BeamIdType::AppId => Self::AppId(AppId::new(id)?),
            BeamIdType::ProxyId => Self::ProxyId(ProxyId::new(id)?),
            BeamIdType::BrokerId => Self::BrokerId(BrokerId::new(id)?),
        };
        Ok(res)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum AppOrProxyId {
    AppId(AppId),
    ProxyId(ProxyId),
}

impl Display for AppOrProxyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.value())
    }
}

impl BeamId for AppOrProxyId {
    fn value(&self) -> &String {
        match self {
            AppOrProxyId::AppId(e) => e.value(),
            AppOrProxyId::ProxyId(e) => e.value(),
        }
    }

    fn new(id: &str) -> Result<Self,SamplyBrokerError> {
        let res = match AppId::str_has_type(id)? { // TODO: Better use trait
            BeamIdType::AppId => Self::AppId(AppId::new(id)?),
            BeamIdType::ProxyId => Self::ProxyId(ProxyId::new(id)?),
            BeamIdType::BrokerId => { return Err(SamplyBrokerError::InvalidBeamId("An AppOrProxyId cannot carry a BrokerId.".into())); },
        };
        Ok(res)
    }
}

impl From<ProxyId> for AppOrProxyId {
    fn from(id: ProxyId) -> Self {
        AppOrProxyId::ProxyId(id)
    }
}

impl From<AppId> for AppOrProxyId {
    fn from(id: AppId) -> Self {
        AppOrProxyId::AppId(id)
    }
}