#![allow(unused_imports)]

use beam_id2::{BeamId, BeamIdTrait};
use crypto_jwt::extract_jwt;
use errors::SamplyBrokerError;
use static_init::dynamic;
use tracing::debug;

use std::{time::Duration, ops::Deref, fmt::Display};

use rand::Rng;
use serde::{Deserialize, Serialize, de::Visitor};
use std::{collections::HashMap, str::FromStr};
use uuid::Uuid;

pub type MsgId = MyUuid;
pub type MsgType = String;
pub type TaskResponse = String;

mod traits;
pub mod logger;
pub mod crypto;
pub mod crypto_jwt;
pub mod errors;

pub mod config;
pub mod config_shared;
// #[cfg(feature = "config-for-central")]
pub mod config_central;
// #[cfg(feature = "config-for-proxy")]
pub mod config_proxy;

pub mod middleware;
pub mod http_proxy;
// pub mod beam_id;
pub mod beam_id2;

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

#[derive(Clone,Debug,Serialize,Deserialize, PartialEq)]
pub struct MsgSigned<M: Msg> {
    pub msg: M,
    pub sig: String
}

impl<M: Msg> MsgSigned<M> {
    pub async fn verify(&self) -> Result<(), SamplyBrokerError> {
        // Signature valid?
        let (public, _, content) 
            = extract_jwt(&self.sig).await?;

        // Message content matches token?
        let val = serde_json::to_value(&self.msg)
        .expect("Internal error: Unable to interpret already parsed message to JSON Value.");
        if content.custom != val {
            return Err(SamplyBrokerError::RequestValidationFailed);
        }

        // From field matches CN in certificate?
        if public.client != *self.get_from() {
            return Err(SamplyBrokerError::RequestValidationFailed);
        }
        debug!("Message has been verified succesfully.");
        Ok(())
    }
}

#[dynamic]
pub static EMPTY_VEC_CLIENTID: Vec<BeamId> = Vec::new();

#[derive(Serialize,Deserialize,Debug)]
pub struct MsgEmpty {
    pub id: MsgId,
    pub from: BeamId,
}

impl Msg for MsgEmpty {
    fn get_id(&self) -> &MsgId {
        &self.id
    }

    fn get_from(&self) -> &BeamId {
        &self.from
    }

    fn get_to(&self) -> &Vec<BeamId> {
        &EMPTY_VEC_CLIENTID
    }
}

pub trait Msg: Serialize {
    fn get_id(&self) -> &MsgId;
    fn get_from(&self) -> &BeamId;
    fn get_to(&self) -> &Vec<dyn BeamIdTrait>;
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

impl<M: Msg> Msg for MsgSigned<M> {
    fn get_id(&self) -> &MsgId {
        self.msg.get_id()
    }

    fn get_from(&self) -> &BeamId {
        self.msg.get_from()
    }

    fn get_to(&self) -> &Vec<BeamId> {
        self.msg.get_to()
    }
}

impl Msg for MsgTaskRequest {
    fn get_id(&self) -> &MsgId {
        &self.id
    }

    fn get_from(&self) -> &BeamId {
        &self.from
    }

    fn get_to(&self) -> &Vec<BeamId> {
        &self.to
    }
}

impl Msg for MsgTaskResult {
    fn get_id(&self) -> &MsgId {
        &self.id
    }

    fn get_from(&self) -> &BeamId {
        &self.from
    }

    fn get_to(&self) -> &Vec<BeamId> {
        &self.to
    }
}

// impl From<MsgSigned<MsgTaskRequest>> for MsgTaskRequest {
//     fn from(x: MsgSigned<MsgTaskRequest>) -> Self {
//         x.msg
//     }
// }

// impl From<MsgSigned<MsgTaskResult>> for MsgTaskResult {
//     fn from(x: MsgSigned<MsgTaskResult>) -> Self {
//         x.msg
//     }
// }

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MsgTaskRequest {
    pub id: MsgId,
    pub from: BeamId,
    pub to: Vec<BeamId>,
    pub task_type: MsgType,
    pub body: String,
    // pub expire: SystemTime,
    pub failure_strategy: FailureStrategy,
    #[serde(skip)]
    pub results: HashMap<BeamIdTrait,MsgSigned<MsgTaskResult>>,
}
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EncryptedMsgTaskRequest {
    pub id: MsgId,
    pub from: BeamId,
    pub to: Vec<BeamId>,
    //auth
    pub task_type: Option<MsgType>,
    pub body: Option<String>,
    // pub expire: SystemTime,
    pub failure_strategy: Option<FailureStrategy>,
    pub encrypted: String,
    pub encryption_keys: Vec<Option<String>>,
    #[serde(skip)]
    pub results: HashMap<BeamId,MsgTaskResult>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct MsgTaskResult {
    pub id: MsgId,
    pub from: BeamId, // was: worker_id
    pub to: Vec<BeamId>,
    pub task: MsgId,
    pub result: WorkResult,
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

impl<M> HasWaitId<MsgId> for MsgSigned<M> where M: HasWaitId<MsgId> + Msg {
    fn get_wait_id(&self) -> MsgId {
        self.msg.get_wait_id()
    }
}

impl MsgTaskRequest {
    fn new(
        from: BeamId,
        to: Vec<BeamId>,
        task_type: MsgType,
        body: String,
        failure_strategy: FailureStrategy,
    ) -> Self {
        MsgTaskRequest {
            id: MsgId::new(),
            from,
            to,
            task_type,
            body,
            failure_strategy,
            results: HashMap::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MsgPing {
    id: MsgId,
    from: BeamId,
    to: Vec<BeamId>,
    nonce: [u8; 16]
}

impl MsgPing {
    pub fn new(from: BeamId, to: BeamId) -> Self {
        let mut nonce = [0;16];
        openssl::rand::rand_bytes(&mut nonce)
            .expect("Critical Error: Failed to generate random byte array.");
        MsgPing { id: MsgId::new(), from, to: vec![to], nonce }
    }
}

impl Msg for MsgPing {
    fn get_id(&self) -> &MsgId {
        &self.id
    }

    fn get_from(&self) -> &BeamId {
        &self.from
    }

    fn get_to(&self) -> &Vec<BeamId> {
        &self.to
    }
}

pub fn generate_example_tasks(client1_id: Option<BeamIdTrait>) -> HashMap<MsgId, MsgTaskRequest> {
    let mut tasks: HashMap<MsgId, MsgTaskRequest> = HashMap::new();
    let client1 = client1_id.unwrap_or_default();
    let client2 = BeamIdTrait::random();

    let task_for_clients_1_2 = MsgTaskRequest::new(
        client1.clone(),
        vec![client1.clone(), client2.clone()],
        "My important task".to_string(),
        "This task is for client1 and client2".to_string(),
        FailureStrategy::Retry { backoff_millisecs: 1000, max_tries: 5 },
    );

    let response_by_client1 = MsgTaskResult {
        id: MsgId::new(),
        from: client1.clone(),
        to: vec![client1.clone()],
        task: task_for_clients_1_2.id,
        result: crate::WorkResult::Succeeded("All done!".to_string()),
    };
    let response_by_client2 = MsgTaskResult {
        id: MsgId::new(),
        from: client2,
        to: vec![client1],
        task: task_for_clients_1_2.id,
        result: crate::WorkResult::PermFailed("Unable to complete".to_string()),
    };
    tasks.insert(task_for_clients_1_2.id, task_for_clients_1_2);
    let task_in_map = tasks.values_mut().next().unwrap(); // only used in testing
    for result in [response_by_client1, response_by_client2] {
        let result = MsgSigned{
            msg: result,
            sig: String::from("just_an_example"),
        };
        task_in_map.results.insert(result.msg.from.clone(), result);
    }
    tasks
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
