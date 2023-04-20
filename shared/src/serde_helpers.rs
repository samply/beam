
pub mod serialize_time {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use fundu::parse_duration;
    use serde::{self, Deserialize, Deserializer, Serializer};
    use tracing::{debug, error, warn, trace};

    pub fn serialize<S>(time: &SystemTime, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let ttl = match time.duration_since(SystemTime::now()) {
            Ok(v) => v,
            Err(e) => {
                error!("Internal Error: Tried to serialize a task which should have expired and expunged from memory {} seconds ago. Will return TTL=0. Cause: {}", e.duration().as_secs(), e);
                Duration::ZERO
            }
        };
        s.serialize_str(&ttl.as_secs().to_string())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SystemTime, D::Error>
    where
        D: Deserializer<'de>,
    {
        let duration = &String::deserialize(deserializer)?;
        let ttl = parse_duration(&duration).map_err(serde::de::Error::custom)?;
        let expire = SystemTime::now() + ttl;
        trace!("Deserialized {:?} to time {:?}", duration, expire);
        Ok(expire)
    }
}

// https://github.com/serde-rs/json/issues/360#issuecomment-330095360
pub mod serde_base64 {
    use serde::{Serializer, de, ser, Deserialize, Deserializer};
    use openssl::base64;

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        serializer.serialize_str(&base64::encode_block(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
        where D: Deserializer<'de>
    {
        base64::decode_block(<&str>::deserialize(deserializer)?).map_err(de::Error::custom)
    }

    pub mod nested {
        use serde::ser::SerializeSeq;

        use super::{ser, de, Serializer, Deserializer, base64};

        pub fn serialize<S>(bytes: &[Vec<u8>], serializer: S) -> Result<S::Ok, S::Error>
            where S: Serializer
        {
            let mut seq_serializer = serializer.serialize_seq(Some(bytes.len()))?;
            for byte_seq in bytes {
                seq_serializer.serialize_element(&base64::encode_block(&byte_seq))?;
            }
            seq_serializer.end()
        }

        pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<Vec<u8>>, D::Error>
            where D: Deserializer<'de>
        {
            <Vec<&str> as serde::Deserialize>::deserialize(deserializer)?
                .into_iter()
                .map(base64::decode_block)
                .collect::<Result<Vec<_>, _>>()
                .map_err(de::Error::custom)
        }
    }
}
