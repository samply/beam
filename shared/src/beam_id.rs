use std::fmt::Display;

use rand::Rng;
use serde::{Serialize, Deserialize, de::Visitor};

use crate::errors::SamplyBrokerError;

// #[derive(Serialize,Debug,Clone,Eq,Hash,PartialEq)]
// // #[serde(transparent)]
// enum BeamIdInner {
//     AppId(IdString),
//     NodeId(IdString),
//     BrokerId(IdString)
// }

// pub enum BeamIdType {
//     AppId,
//     NodeId,
//     BrokerId
// }

// pub struct BeamId<BeamIdType>(BeamIdInner);

// impl<BeamIdType> BeamId<BeamIdType> {
//     pub fn new(id_type: BeamIdType, id: &str) -> Result<Self, SamplyBrokerError> {
//         let id_str = IdString::new(id)?;

//         Ok(BeamId(id_str))
//     }
// }

#[derive(Serialize,Debug,Clone,Eq,Hash,PartialEq)]
struct IdString {
    id: String,
}

impl IdString {
    fn new(id: &str) -> Result<Self, SamplyBrokerError> {
        if Self::is_valid(id) {
            Ok(Self { id: id.into() })
        } else {
            Err(SamplyBrokerError::InvalidClientIdString(id.into()))
        }
    }

    fn random() -> Self {
        const LENGTH: u8 = 8;
        const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
        const SUFFIX: &str = ".randomclientid";
        let mut rng = rand::thread_rng();
        let mut random_id: String = (0..=LENGTH)
            .map(|_| {
                let idx = rng.gen_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect();
        random_id.push_str(SUFFIX);
        Self::new(&random_id)
            .expect("Internal Error: ClientId::random() generated invalid client id. This should not happen")
    }

    fn is_valid(id: &str) -> bool {
        if ! id.contains('.') {
            return false;
        }
        for char in id.chars() {
            if !(char.is_alphanumeric() || char == '.' || char == '-'){
                return false;
            }
        }
        true
    }
}

impl Display for IdString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.id)
    }
}

impl Default for IdString {
    fn default() -> Self {
        Self::random()
    }
}

impl TryFrom<String> for IdString {
    type Error = SamplyBrokerError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl TryFrom<&str> for IdString {
    type Error = SamplyBrokerError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for IdString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de> {
        deserializer.deserialize_str(IdStringVisitor)
    }
}

struct IdStringVisitor;

impl<'de> Visitor<'de> for IdStringVisitor {
    type Value = IdString;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(formatter, "string of lower-case letters and/or numbers and at least one '.' separator")
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        IdString::new(v)
            .map_err(|_| serde::de::Error::custom("Invalid ID string"))
    }
}