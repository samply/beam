use serde::{Serialize, Deserialize, de::DeserializeOwned};
use serde_json::Value;
use uuid::Uuid;
use crate::AddressingId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct MsgId(Uuid);

impl MsgId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl std::fmt::Display for MsgId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRequest<T> {
    pub id: MsgId,
    pub from: AddressingId,
    pub to: Vec<AddressingId>,
    #[serde(
        with = "serde_string",
        bound(serialize = "T: Serialize + 'static", deserialize = "T: DeserializeOwned + 'static")
    )]
    pub body: T,
    pub ttl: String,
    pub failure_strategy: FailureStrategy,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult<T> {
    pub from: AddressingId,
    pub to: Vec<AddressingId>,
    pub task: MsgId,
    pub status: WorkStatus,
    #[serde(
        with = "serde_string",
        bound(serialize = "T: Serialize + 'static", deserialize = "T: DeserializeOwned + 'static")
    )]
    pub body: T,
    pub metadata: Value,
}

#[cfg(feature = "sockets")]
#[derive(Debug, Serialize, Deserialize)]
pub struct SocketTask {
    pub from: AddressingId,
    pub to: Vec<AddressingId>,
    pub ttl: String,
    pub id: MsgId,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FailureStrategy {
    Discard,
    Retry {
        backoff_millisecs: usize,
        max_tries: usize,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WorkStatus {
    Claimed,
    Succeeded,
    TempFailed,
    PermFailed,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MsgEmpty {
    pub from: AddressingId,
}

mod serde_string {

    use super::RawString;
    use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};
    use std::any::TypeId;

    pub fn serialize<S, T: Serialize + 'static>(json: &T, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if TypeId::of::<T>() == TypeId::of::<RawString>() {
            json.serialize(serializer)
        } else {
            serializer.serialize_str(&serde_json::to_string(json).map_err(serde::ser::Error::custom)?)
        }
    }

    pub fn deserialize<'de, D, T: DeserializeOwned + 'static>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
    {
        if TypeId::of::<T>() == TypeId::of::<RawString>() {
            T::deserialize(deserializer)
        } else {
            serde_json::from_str(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
        }
    }
}

/// Can be used to extract the raw String as sent by the beam proxy without deserializing it into some json value
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RawString(pub String);

impl RawString {
    pub fn into_string(self) -> String {
        self.0
    }
}

impl<T: Into<String>> From<T> for RawString {
    fn from(value: T) -> Self {
        Self(value.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_serialize_and_deserialize<T: From<&'static str> + PartialEq + Serialize + DeserializeOwned + std::fmt::Debug + 'static>() {
        use crate::AppId;
        #[cfg(feature = "strict-ids")]
        crate::set_broker_id("broker.samply.de".to_string());
        let from = AppId::new_unchecked("test.broker.samply.de").into();
        let task = TaskRequest {
            id: MsgId::new(),
            from,
            to: vec![],
            body: <T>::from("asdf"),
            ttl: "10s".to_string(),
            failure_strategy: FailureStrategy::Discard,
            metadata: Value::Null,
        };
        assert_eq!(serde_json::from_str::<TaskRequest<T>>(&serde_json::to_string(&task).unwrap()).unwrap().body, task.body);
    }

    #[test]
    fn test_raw_string() {
        test_serialize_and_deserialize::<RawString>();
        test_serialize_and_deserialize::<String>();
    }
}
