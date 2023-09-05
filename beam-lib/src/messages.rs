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
    #[serde(with = "serde_string", bound(serialize = "T: Serialize", deserialize = "T: DeserializeOwned"))]
    pub body: T,
    pub ttl: String,
    pub failure_strategy: FailureStrategy,
    pub metadata: Value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult<T> {
    pub from: AddressingId,
    pub to: Vec<AddressingId>,
    pub task: MsgId,
    pub status: WorkStatus,
    #[serde(with = "serde_string", bound(serialize = "T: Serialize", deserialize = "T: DeserializeOwned"))]
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
}


#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FailureStrategy {
    Discard,
    Retry {
        backoff_millisecs: usize,
        max_tries: usize,
    }
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
    use serde::{Serialize, Serializer, Deserializer, Deserialize, de::DeserializeOwned};

    pub fn serialize<S, T: Serialize>(json: &T, serializer: S) -> Result<S::Ok, S::Error>
        where S: Serializer
    {
        serializer.serialize_str(&serde_json::to_string(json).map_err(serde::ser::Error::custom)?)
    }

    pub fn deserialize<'de, D, T: DeserializeOwned>(deserializer: D) -> Result<T, D::Error>
        where D: Deserializer<'de>
    {
        serde_json::from_str(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
