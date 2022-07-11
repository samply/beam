use std::{ops::Deref, fmt::Display};

use crate::{errors::SamplyBrokerError, config};

#[derive(PartialEq)]
pub enum BeamIdType {
    AppId,
    NodeId,
    BrokerId
}

pub trait BeamIdTrait: Display + Sized{
    fn str_has_type(value: &str) -> Result<BeamIdType,SamplyBrokerError> {
        let domain = config::CONFIG_SHARED.broker_domain;
        let split = value.split('.').rev();
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
            return Ok(BeamIdType::NodeId);
        }
        check_valid_id_part(part.unwrap())?;
        if let Some(s) = split.next() {
            return Err(SamplyBrokerError::InvalidBeamId(format!("Beam ID must not continue left of AppID part: {s}")));
        }
        Ok(BeamIdType::AppId)
    }
    fn has_type(&self) -> BeamIdType { // This is for &self, so we can assume the existing ID is correct
        Self::str_has_type(&self.value())
            .expect("Internal error: has_type() on an existing ID has returned an error")
    }
    fn value(&self) -> String;
    fn new(id: &str) -> Result<Self,SamplyBrokerError>;
}

fn check_valid_id_part(id: &str) -> Result<(),SamplyBrokerError> {
    for char in id.chars() {
        if !(char.is_alphanumeric() || char == '-'){
            return Err(SamplyBrokerError::InvalidBeamId(format!("Invalid Beam ID element: {id}")));
        }
    }
    Ok(())
}

// impl Deref for dyn BeamId {
//     type Target = String;

//     fn deref(&self) -> &Self::Target {
//         &self.value()
//     }
// }

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

pub struct AppId(String);
impl BeamIdTrait for AppId {
    fn value(&self) -> String {
        self.0
    }
    fn new(id: &str) -> Result<Self,SamplyBrokerError> {
        if Self::str_has_type(id)? != BeamIdType::AppId {
            return Err(SamplyBrokerError::InvalidBeamId(format!("{id} is not a valid AppID")));
        }
        Ok(AppId(id.to_string()))
    }
}

pub struct BeamId(dyn BeamIdTrait);
pub struct ProxyId(String);
impl BeamIdTrait for ProxyId {
    fn value(&self) -> String {
        self.0
    }

    fn new(id: &str) -> Result<ProxyId, SamplyBrokerError> {
        Ok(ProxyId(String::from(id)))
    }
}
pub struct BrokerId(String);
impl BeamIdTrait for BrokerId {
    fn value(&self) -> String {
        self.0
    }
    fn new(id: &str) -> Result<BrokerId, SamplyBrokerError> {
        Ok(BrokerId(String::from(id)))
    }
}


// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn broker_id() {
//         let value = "broker23.beam.samply.de";
//         let res = BrokerId::try_from(value);
//         assert!(res.is_ok());
//         assert_eq!(res.unwrap(), value);
//     }
// }