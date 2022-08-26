use std::{ops::Deref, fmt::Display, hash::Hash, str::FromStr};

use serde::{Serialize, Deserialize, de::Visitor};
use once_cell::sync::OnceCell;

use crate::{errors::SamplyBeamError, config};

static BROKER_ID: OnceCell<String> = OnceCell::new();

#[derive(PartialEq,Debug)]
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
    fn get_broker_id() -> Option<&'static String> {
        BROKER_ID.get()
    }
    fn set_broker_id(domain: &String) {
        let res = BROKER_ID.set(domain.clone());
        if let Err(value) = res {
            assert_eq!(*domain, value, "Tried to initialize broker_id with two different values");
        }
    }
    fn str_has_type(value: &str) -> Result<BeamIdType,SamplyBeamError> {
        let broker_id = BROKER_ID.get();
        debug_assert!(broker_id.is_some(), "BeamId::str_has_type() called but broker_id not initialized");
        let broker_id = broker_id.unwrap();
        // Broker
        let rest = value.strip_suffix(broker_id.as_str());
        if rest.is_none() {
            return Err(SamplyBeamError::InvalidBeamId(format!("Beam ID must end with {}", broker_id)));
        }
        let mut split = rest.unwrap().split('.').rev();
        let part = split.nth(1);
        if part.is_none() || part.unwrap().is_empty() {
            return Ok(BeamIdType::BrokerId);
        }
        check_valid_id_part(part.unwrap())?;
        let part = split.next();
        if part.is_none() {
            return Ok(BeamIdType::ProxyId);
        }
        check_valid_id_part(part.unwrap())?;
        if let Some(s) = split.next() {
            return Err(SamplyBeamError::InvalidBeamId(format!("Beam ID must not continue left of AppID part: {s}")));
        }
        Ok(BeamIdType::AppId)
    }
    fn has_type(&self) -> BeamIdType { // This is for &self, so we can assume the existing ID is correct
        Self::str_has_type(self.value()).unwrap()
    }
    fn value(&self) -> &String;
    fn new(id: &str) -> Result<Self,SamplyBeamError>;
    fn can_be_signed_by<B: BeamId>(&self, other_id: &B) -> bool {
        return self.value().ends_with(other_id.value());
    }
}

fn check_valid_id_part(id: &str) -> Result<(),SamplyBeamError> {
    for char in id.chars() {
        if !(char.is_alphanumeric() || char == '-'){
            return Err(SamplyBeamError::InvalidBeamId(format!("Invalid Beam ID element: {id}")));
        }
    }
    Ok(())
}

impl Display for AppId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.value())
    }
}

impl Display for ProxyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.value())
    }
}

impl Display for BrokerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.value())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct AppId(String);
impl BeamId for AppId {
    fn value(&self) -> &String {
        &self.0
    }

    fn new(id: &str) -> Result<Self,SamplyBeamError> {
        let given_type = Self::str_has_type(id)?;
        if given_type != BeamIdType::AppId {
            return Err(SamplyBeamError::InvalidBeamId(format!("{id} is a {given_type}, not an AppId.")));
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

    fn new(id: &str) -> Result<Self,SamplyBeamError> {
        let given_type = Self::str_has_type(id)?;
        if given_type != BeamIdType::ProxyId {
            return Err(SamplyBeamError::InvalidBeamId(format!("{id} is a {given_type}, not a ProxyId.")));
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

    fn new(id: &str) -> Result<Self,SamplyBeamError> {
        let given_type = Self::str_has_type(id)?;
        if given_type != BeamIdType::BrokerId {
            return Err(SamplyBeamError::InvalidBeamId(format!("{id} is a {given_type}, not a BrokerId.")));
        }
        Ok(Self(id.to_string()))
    }
}

impl AppId {
    pub fn proxy_id(&self) -> ProxyId {
        // just cut off text until first '.'
        let first_dot = self.0.find('.').unwrap(); // always exists in an AppId
        let mut shortened = self.0.clone();
        shortened.replace_range(..first_dot+1, "");
        assert_eq!(ProxyId::str_has_type(&shortened).unwrap(), BeamIdType::ProxyId);
        ProxyId(shortened)
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
        f.write_str(self.value())
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

    fn new(id: &str) -> Result<Self,SamplyBeamError> {
        let res = match AppId::str_has_type(id)? { // TODO: Better use trait
            BeamIdType::AppId => Self::AppId(AppId::new(id)?),
            BeamIdType::ProxyId => Self::ProxyId(ProxyId::new(id)?),
            BeamIdType::BrokerId => Self::BrokerId(BrokerId::new(id)?),
        };
        Ok(res)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum AppOrProxyId {
    AppId(AppId),
    ProxyId(ProxyId),
}

impl Display for AppOrProxyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.value())
    }
}

impl BeamId for AppOrProxyId {
    fn value(&self) -> &String {
        match self {
            AppOrProxyId::AppId(e) => e.value(),
            AppOrProxyId::ProxyId(e) => e.value(),
        }
    }

    fn new(id: &str) -> Result<Self,SamplyBeamError> {
        let res = match AppId::str_has_type(id)? { // TODO: Better use trait
            BeamIdType::AppId => Self::AppId(AppId::new(id)?),
            BeamIdType::ProxyId => Self::ProxyId(ProxyId::new(id)?),
            BeamIdType::BrokerId => { return Err(SamplyBeamError::InvalidBeamId("An AppOrProxyId cannot carry a BrokerId.".into())); },
        };
        Ok(res)
    }
}

impl TryFrom<&AppOrProxyId> for AppId {
    type Error = SamplyBeamError;

    fn try_from(id: &AppOrProxyId) -> Result<Self, Self::Error> {
        match id {
            AppOrProxyId::AppId(e) => AppId::new(e.value()),
            AppOrProxyId::ProxyId(e) => Err(SamplyBeamError::InvalidBeamId(format!("Not an AppId: {}",e)))
        }
    }
}
impl TryFrom<&AppOrProxyId> for ProxyId {
    type Error = SamplyBeamError;
    fn try_from(id: &AppOrProxyId) -> Result<Self, Self::Error> {
        match id {
            AppOrProxyId::ProxyId(e) => ProxyId::new(e.value()),
            AppOrProxyId::AppId(e) => Err(SamplyBeamError::InvalidBeamId(format!("Not a ProxyId: {}",e)))
        }
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

impl From<&AppId> for AppOrProxyId {
    fn from(id: &AppId) -> Self {
        AppOrProxyId::AppId(id.clone())
    }
}

impl FromStr for AppId {
    type Err = SamplyBeamError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        AppId::new(s)
    }
}

impl PartialEq<AppId> for AppOrProxyId {
    fn eq(&self, other: &AppId) -> bool {
        match self {
            Self::AppId(id) => id == other,
            Self::ProxyId(_) => false,
        }
    }
}

impl PartialEq<&String> for AppOrProxyId {
    fn eq(&self, other: &&String) -> bool {
        self.value() == *other
    }
}

impl PartialEq<ProxyId> for AppOrProxyId {
    fn eq(&self, other: &ProxyId) -> bool {
        match self {
            Self::ProxyId(id) => id == other,
            Self::AppId(_) => false,
        }
    }
}

impl<'de> Deserialize<'de> for AppOrProxyId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de> {
        deserializer.deserialize_str(AppOrProxyIdVisitor)
    }
}

impl Serialize for AppOrProxyId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer {
        serializer.serialize_str(self.value())
    }
}

struct AppOrProxyIdVisitor;

impl<'de> Visitor<'de> for AppOrProxyIdVisitor {
    type Value = AppOrProxyId;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(formatter, "AppId = <app_id>.<proxy_id>.<broker_id> or ProxyId = <proxy_id>.<broker_id>")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let t = AppId::str_has_type(v)
            .map_err(|e| serde::de::Error::custom(format!("Invalid Beam ID \"{v}\": {e}")))?;
        match t {
            BeamIdType::AppId => Ok(AppOrProxyId::AppId(AppId::new(v).unwrap())),
            BeamIdType::ProxyId => Ok(AppOrProxyId::ProxyId(ProxyId::new(v).unwrap())),
            BeamIdType::BrokerId => {
                Err(serde::de::Error::custom("Expected AppOrProxyId, got BrokerId."))
            }
        }
    }
}

pub fn app_to_broker_id(app_id: &str) -> Result<String,SamplyBeamError> {
    let mut shortened = app_id.to_string();
    // cut off text until second '.'
    for _ in 1..=2 {
        let first_dot = 
            shortened.find('.')
            .ok_or_else(|| SamplyBeamError::InvalidBeamId(format!("{app_id} is not a valid AppId. An AppId has the form <app>.<proxy>.<broker>, see documentation.")))?;
        shortened.replace_range(..first_dot+1, "");
    }
    Ok(shortened)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_broker_id(broker_id: &str) {
        if let Err(prev) = BROKER_ID.set(broker_id.into()) {
            if prev != broker_id {
                panic!("Tried to initialize BROKER_ID with two different values.");
            }
        }
    }

    #[test]
    fn test_str_has_type() {
        init_broker_id("broker.samply.de");
        assert_eq!(BrokerId::str_has_type("broker.samply.de").unwrap(), BeamIdType::BrokerId);
        assert_eq!(BrokerId::str_has_type("proxy23.broker.samply.de").unwrap(), BeamIdType::ProxyId);
        assert_eq!(BrokerId::str_has_type("app12.proxy23.broker.samply.de").unwrap(), BeamIdType::AppId);
        assert!(BrokerId::str_has_type("roker.samply.de").is_err());
        assert!(BrokerId::str_has_type("moreString.app12.proxy23.broker.samply.de").is_err());
    }

    #[test]
    fn test_appid_brokerid() {
        let app_id_str = "app.proxy1.broker.samply.de";
        let actual_broker_id_from_str = app_to_broker_id(app_id_str).unwrap();
        assert_eq!("broker.samply.de", actual_broker_id_from_str);

        init_broker_id(&actual_broker_id_from_str);
        let app_id = AppId::new(app_id_str).unwrap();
        let actual_broker_id_str = app_to_broker_id(&app_id.to_string()).unwrap();
        assert_eq!(actual_broker_id_str, actual_broker_id_from_str);

        let actual_broker_id = BrokerId::new(&actual_broker_id_str).unwrap();
        assert_eq!(actual_broker_id.to_string(), actual_broker_id_str);
    }
    #[test]
    fn try_from_app_or_proxy_id() {
        let app_id_str = "app.proxy1.broker.samply.de";
        let broker_id = app_to_broker_id(app_id_str).unwrap();

        AppId::set_broker_id(&broker_id);
        let app_id = AppId::new(app_id_str).unwrap();
        let aop_id_app: AppOrProxyId = app_id.clone().into();
        let app_result = AppId::try_from(&aop_id_app).unwrap();
        assert_eq!(app_id, app_result);

        ProxyId::set_broker_id(&broker_id);
        let proxy_id = app_id.proxy_id();
        let aop_id_proxy: AppOrProxyId = proxy_id.clone().into();
        let proxy_result = ProxyId::try_from(&aop_id_proxy).unwrap();
        assert_eq!(proxy_id, proxy_result);
    }
}
