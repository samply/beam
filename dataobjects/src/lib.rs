//#![allow(unused_imports)]

use beam_id::{BeamId, AppId, AppOrProxyId};
use serde_json::{Value, json};
use tracing::debug;

use std::{time::Duration, ops::Deref, fmt::Display};

use rand::Rng;
use serde::{Deserialize, Serialize, de::Visitor};
use std::{collections::HashMap, str::FromStr};
use uuid::Uuid;

pub type MsgId = MyUuid;
pub type MsgType = String;
pub type TaskResponse = String;

pub mod errors;
pub mod beam_id;

#[derive(Debug,Serialize,Deserialize,Clone,Copy,PartialEq,Eq,Hash)]
pub struct MyUuid(Uuid);
impl MyUuid {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for MyUuid {
    fn default() -> Self {
        Self::new()
    }
}
impl Deref for MyUuid {
    type Target = Uuid;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl From<Uuid> for MyUuid {
    fn from(uuid: Uuid) -> Self {
        MyUuid(uuid)
    }
}
impl TryFrom<&str> for MyUuid {
    type Error = uuid::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let parsed = Uuid::from_str(value)?;
        Ok(Self(parsed))
    }
}

impl Display for MyUuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Deserialize)]
pub struct HowLongToBlockAsIntegers {
    poll_timeout: Option<u64>,
    poll_count: Option<u16>,
}

#[derive(PartialEq, Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "lowercase")]
pub enum WorkResult {
    Unclaimed,
    TempFailed(TaskResponse),
    PermFailed(TaskResponse),
    Succeeded(TaskResponse),
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FailureStrategy {
    Discard,
    Retry {
        backoff_millisecs: usize,
        max_tries: usize,
    }, // backoff for Duration and try max. times
}

#[derive(Serialize, Deserialize, Debug)]
pub struct HowLongToBlock {
    pub poll_timeout: Option<Duration>,
    pub poll_count: Option<u16>,
}

//#[dynamic]
pub static EMPTY_VEC_APPORPROXYID: Vec<AppOrProxyId> = Vec::new();


#[derive(Clone,Debug,Serialize,Deserialize, PartialEq)]
pub struct MsgSigned<M: Msg> {
    pub msg: M,
    pub sig: String
}
impl<M> HasWaitId<MsgId> for MsgSigned<M> where M: HasWaitId<MsgId> + Msg {
    fn get_wait_id(&self) -> MsgId {
        self.msg.get_wait_id()
    }
}
#[derive(Serialize,Deserialize,Debug)]
pub struct MsgEmpty {
    pub id: MsgId,
    pub from: AppOrProxyId,
}

impl Msg for MsgEmpty {
    fn get_id(&self) -> &MsgId {
        &self.id
    }

    fn get_from(&self) -> &AppOrProxyId {
        &self.from
    }

    fn get_to(&self) -> &Vec<AppOrProxyId> {
        &EMPTY_VEC_APPORPROXYID
    }

    fn get_metadata(&self) -> &Value {
        &json!(null)
    }
}

pub trait Msg: Serialize {
    fn get_id(&self) -> &MsgId;
    fn get_from(&self) -> &AppOrProxyId;
    fn get_to(&self) -> &Vec<AppOrProxyId>;
    fn get_metadata(&self) -> &Value;
}

pub trait MsgWithBody : Msg{
    // fn get_body(&self) -> &str;
}
impl MsgWithBody for MsgTaskRequest {
    // fn get_body(&self) -> &str {
    //     &self.body
    // }
}
impl MsgWithBody for MsgTaskResult {
    // fn get_body(&self) -> &str {
    //     self.get_body()
    // }
}

impl Msg for MsgTaskRequest {
    fn get_id(&self) -> &MsgId {
        &self.id
    }

    fn get_from(&self) -> &AppOrProxyId {
        &self.from
    }

    fn get_to(&self) -> &Vec<AppOrProxyId> {
        &self.to
    }

    fn get_metadata(&self) -> &Value {
        &self.metadata
    }
}

impl Msg for MsgTaskResult {
    fn get_id(&self) -> &MsgId {
        &self.id
    }

    fn get_from(&self) -> &AppOrProxyId {
        &self.from
    }

    fn get_to(&self) -> &Vec<AppOrProxyId> {
        &self.to
    }

    fn get_metadata(&self) -> &Value {
        &self.metadata
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MsgTaskRequest {
    pub id: MsgId,
    pub from: AppOrProxyId,
    pub to: Vec<AppOrProxyId>,
    pub body: String,
    // pub expire: SystemTime,
    pub failure_strategy: FailureStrategy,
    //TODO --> Combined Object to broker
    #[serde(skip)]
    pub results: HashMap<AppOrProxyId,MsgSigned<MsgTaskResult>>,
    pub metadata: Value
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EncryptedMsgTaskRequest {
    pub id: MsgId,
    pub from: AppOrProxyId,
    pub to: Vec<AppOrProxyId>,
    //auth
    pub body: Option<String>,
    // pub expire: SystemTime,
    pub failure_strategy: Option<FailureStrategy>,
    pub encrypted: String,
    pub encryption_keys: Vec<Option<String>>,
    #[serde(skip)]
    pub results: HashMap<AppOrProxyId,MsgTaskResult>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MsgTaskResult {
    pub id: MsgId,
    pub from: AppOrProxyId, // was: worker_id
    pub to: Vec<AppOrProxyId>,
    pub task: MsgId,
    pub result: WorkResult,
    pub metadata: Value,
}

pub trait HasWaitId<T> {
    fn get_wait_id(&self) -> T;
}

impl HasWaitId<MsgId> for MsgTaskRequest {
    fn get_wait_id(&self) -> MsgId {
        self.id
    }
}

impl HasWaitId<MsgId> for MsgTaskResult {
    fn get_wait_id(&self) -> MsgId {
        self.task
    }
}

impl MsgTaskRequest {
    pub fn new(
        from: AppOrProxyId,
        to: Vec<AppOrProxyId>,
        body: String,
        failure_strategy: FailureStrategy,
        metadata: serde_json::Value
    ) -> Self {
        MsgTaskRequest {
            id: MsgId::new(),
            from,
            to,
            body,
            failure_strategy,
            results: HashMap::new(),
            metadata
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MsgPing {
    id: MsgId,
    from: AppOrProxyId,
    to: Vec<AppOrProxyId>,
    nonce: [u8; 16],
    metadata: Value
}

impl MsgPing {
    pub fn new(from: AppOrProxyId, to: AppOrProxyId) -> Self {
        let mut nonce = [0;16];
        openssl::rand::rand_bytes(&mut nonce)
            .expect("Critical Error: Failed to generate random byte array.");
        MsgPing { id: MsgId::new(), from, to: vec![to], nonce, metadata: json!(null) }
    }
}

impl Msg for MsgPing {
    fn get_id(&self) -> &MsgId {
        &self.id
    }

    fn get_from(&self) -> &AppOrProxyId {
        &self.from
    }

    fn get_to(&self) -> &Vec<AppOrProxyId> {
        &self.to
    }

    fn get_metadata(&self) -> &Value {
        &self.metadata
    }
}

pub fn try_read<T>(map: &HashMap<String, String>, key: &str) -> Option<T>
where
    T: FromStr,
{
    map.get(key).and_then(|value| match value.parse() {
        Ok(v) => Some(v),
        Err(_) => None,
    })
}
