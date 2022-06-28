use std::{time::Duration, ops::Deref, fmt::Display};

use serde::{Deserialize, Serialize};
use std::{collections::HashMap, str::FromStr};
use uuid::Uuid;

pub type MsgId = MyUuid;
pub type ClientId = MyUuid;
pub type MsgType = String;
pub type TaskResponse = String;

mod traits;

#[derive(Debug,Serialize,Deserialize,Clone,Copy,PartialEq,Eq,Hash)]
pub struct MyUuid(Uuid);
impl MyUuid {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
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

#[derive(Serialize, Deserialize, Debug, Clone)]
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MsgTaskRequest {
    pub id: MsgId,
    pub to: Vec<ClientId>,
    //auth
    pub task_type: MsgType,
    pub body: String,
    // pub expire: SystemTime,
    pub failure_strategy: FailureStrategy,
    #[serde(skip)]
    pub results: HashMap<ClientId,MsgTaskResult>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct MsgTaskResult {
    pub id: MsgId,
    pub worker_id: ClientId,
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
        self.worker_id
    }
}

impl MsgTaskRequest {
    fn new(
        to: Vec<ClientId>,
        task_type: MsgType,
        body: String,
        failure_strategy: FailureStrategy,
    ) -> Self {
        MsgTaskRequest {
            id: MsgId::new(),
            to,
            task_type,
            body,
            failure_strategy,
            results: HashMap::new(),
        }
    }
}

pub fn generate_example_tasks() -> HashMap<MsgId, MsgTaskRequest> {
    let mut tasks: HashMap<MsgId, MsgTaskRequest> = HashMap::new();
    let my_id = ClientId::new();
    let to = vec![my_id];

    let task = MsgTaskRequest::new(
        to,
        "My important task".to_string(),
        "Much work to do".to_string(),
        FailureStrategy::Retry { backoff_millisecs: 1000, max_tries: 5 },
    );

    let response1 = MsgTaskResult {
        id: MsgId::new(),
        worker_id: my_id,
        task: task.id,
        result: crate::WorkResult::Succeeded("All done!".to_string()),
    };
    let someone_elses_id = Uuid::new_v4();
    let response2 = MsgTaskResult {
        id: MsgId::new(),
        worker_id: someone_elses_id.into(),
        task: task.id,
        result: crate::WorkResult::PermFailed("Unable to complete".to_string()),
    };
    tasks.insert(task.id, task);
    let task_in_map = tasks.values_mut().next().unwrap();
    for result in [response1, response2] {
        task_in_map.results.insert(result.worker_id, result);
    }
    tasks
}

#[cfg(test)]
mod tests {
    use crate::{generate_example_tasks, MsgTaskResult, WorkResult};

    #[test]
    fn check_map() {
        let tasks = generate_example_tasks();
        assert!(tasks.len() == 1);
        let the_task = tasks.values().next().unwrap();
        assert!(the_task.results.len() == 2);
    }

    #[test]
    fn get_failed_responses() {
        let tasks = generate_example_tasks();
        let failed: Vec<&MsgTaskResult> = tasks.values().next().unwrap().results
            .iter()
            .filter(|resp| matches!(resp.1.result, WorkResult::PermFailed(_)))
            .map(|(_, v)| v)
            .collect();

        assert!(failed.len() == 1);
    }

    #[test]
    fn serialize_stuff() {
        let tasks = generate_example_tasks();
        let serialized = serde_json::to_string(&tasks);
        assert!(serialized.is_ok());
        // println!("Tasks and results: {}", serialized.unwrap());
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
