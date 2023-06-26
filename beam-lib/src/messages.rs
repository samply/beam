use serde::{Serialize, Deserialize};
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
struct TaskRequest<T> {
    pub id: MsgId,
    pub from: AddressingId,
    pub to: Vec<AddressingId>,
    pub body: T,
    pub ttl: String,
    pub failure_strategy: FailureStrategy,
    pub metadata: Value
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskResult<T> {
    pub from: AddressingId,
    pub to: Vec<AddressingId>,
    pub task: MsgId,
    pub status: WorkStatus,
    pub body: T,
    pub metadata: Value,
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

#[derive(Debug, Serialize, Deserialize, Clone, Eq, PartialEq)]
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

