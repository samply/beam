use std::{ops::Deref, borrow::Cow, time::{SystemTime, Duration}};

use axum::{response::IntoResponse, Json};
use dashmap::{DashMap, mapref::{multiple::RefMulti, one::Ref}};
use hyper::StatusCode;
use shared::{HasWaitId, MsgId, Msg, MsgSigned, MsgTaskRequest, MsgState, MsgTaskResult, MsgEmpty, MsgSocketRequest, MsgSocketResult, HowLongToBlock, beam_id::AppOrProxyId};
use tokio::{sync::broadcast, time::Instant};
use tracing::warn;

pub trait Task {
    type Result: Msg + Clone;

    fn get_results(&self) -> &Vec<Self::Result>;
    fn push_result(&mut self, result: Self::Result);
    fn is_expired(&self) -> bool;
}

// impl<State: MsgState> Task for MsgTaskRequest<State> {
//     type Result = MsgSigned<MsgTaskResult<State>>;

//     fn get_results(&self) -> &Vec<Self::Result> {
//         &self.results
//     }

//     fn push_result(&mut self, result: Self::Result) {
//         self.results.push(result)
//     }
// }

impl<State: MsgState> Task for MsgSocketRequest<State> {
    type Result = MsgSigned<MsgSocketResult>;

    fn get_results(&self) -> &Vec<Self::Result> {
        &self.result
    }

    fn push_result(&mut self, result: Self::Result) {
        if self.result.len() == 0 {
            self.result.push(result);
        } else {
            self.result[0] = result;
        }
    }

    fn is_expired(&self) -> bool {
        self.expire < SystemTime::now()
    }
}

pub struct TaskManager<T: HasWaitId<MsgId> + Task + Msg> {
    tasks: DashMap<MsgId, MsgSigned<T>>,
    new_tasks: broadcast::Sender<MsgId>,
    deleted_tasks: broadcast::Sender<MsgId>,
    /// Send the index at which the new result for the given Task was inserted
    new_results: DashMap<MsgId, broadcast::Sender<usize>>
}

impl<T: HasWaitId<MsgId> + Task + Msg> TaskManager<T> {
    pub fn new() -> Self {
        // TODO: spawn expire
        let (new_tasks, _) = broadcast::channel(256);
        let (deleted_tasks, _) = broadcast::channel(256);
        Self {
            tasks: Default::default(),
            new_tasks,
            deleted_tasks,
            new_results: Default::default()
        }
    }

    pub fn get_tasks_by(&self, filter: impl Fn(&T) -> bool) -> impl Iterator<Item = impl Deref<Target = MsgSigned<T>> + '_> {
        self.tasks
            .iter()
            .filter(move |entry| filter(&entry.msg))
            .filter(|entry| !entry.msg.is_expired())
    }

    // Once async iterators are stabelized this should be one
    pub async fn wait_for_tasks(&self, block: &HowLongToBlock, filter: impl Fn(&T) -> bool) -> Result<impl Iterator<Item = impl Deref<Target = MsgSigned<T>> + '_>, TaskManagerError> {
        let max_elements = block.wait_count.unwrap_or(u16::MAX) as usize;
        let wait_until = Instant::now() + block
            .wait_time
            .unwrap_or(Duration::from_secs(600));
        let mut new_tasks = self.new_tasks.subscribe();
        let mut deleted_tasks = self.deleted_tasks.subscribe();
        
        let mut num_of_tasks = self
            .get_tasks_by(&filter)
            .count();
        while num_of_tasks < max_elements && Instant::now() < wait_until {
            tokio::select! {
                _ = tokio::time::sleep_until(wait_until) => {
                    break;
                },
                result = new_tasks.recv() => {
                    match result {
                        Ok(id) => {
                            if let Ok(task) = self.get(&id) {
                                if filter(&task.msg) {
                                    num_of_tasks += 1;
                                }
                            }
                        },
                        Err(e) => {
                            warn!("new_tasks channel lagged: {e}");
                            return Err(TaskManagerError::BroadcastBufferOverflow);
                        }
                    }
                },
                result = deleted_tasks.recv() => {
                    match result {
                        Ok(id) => {
                            if let Ok(task) = self.get(&id) {
                                if filter(&task.msg) {
                                    num_of_tasks -= 1;
                                }
                            }
                        },
                        Err(e) => {
                            warn!("delted_tasks channel lagged: {e}");
                            return Err(TaskManagerError::BroadcastBufferOverflow);
                        }
                    }
                },
            }
        }
        Ok(self.get_tasks_by(filter))
    }

    /// This does not check if the requester was the creator of the Task
    pub async fn wait_for_results(&self, task_id: &MsgId, block: &HowLongToBlock, filter: impl Fn(&T::Result) -> bool) -> Result<impl Deref<Target = MsgSigned<T>> + '_, TaskManagerError> {
        let max_elements = block.wait_count.unwrap_or(u16::MAX) as usize;
        let wait_until = Instant::now() + block
            .wait_time
            .unwrap_or(Duration::from_secs(600));

        let mut num_of_results = self.get(task_id)?.msg
            .get_results()
            .iter()
            .filter(|result| filter(result))
            .count();
        let mut new_results = self.new_results.get(task_id).ok_or(TaskManagerError::NotFound)?.subscribe();
        while num_of_results < max_elements && Instant::now() < wait_until {
            tokio::select! {
                _ = tokio::time::sleep_until(wait_until) => {
                    break;
                },
                result = new_results.recv() => {
                    match result {
                        Ok(idx) => {
                            if let Ok(task) = self.get(task_id) {
                                if filter(&task.msg.get_results()[idx]) {
                                    num_of_results += 1;
                                }
                            } else {
                                return Err(TaskManagerError::Gone);
                            }
                        },
                        Err(e) => {
                            warn!("new_results channel lagged: {e}");
                            return Err(TaskManagerError::BroadcastBufferOverflow);
                        }
                    }
                },
            }
        }

        Ok(self.get(task_id).map_err(|_| TaskManagerError::Gone)?)
    }

    pub fn get(&self, task_id: &MsgId) -> Result<impl Deref<Target = MsgSigned<T>> + '_, TaskManagerError> {
        self.tasks.get(task_id).ok_or(TaskManagerError::NotFound)
    }

    /// This will push the result to the given task by its id
    /// Returns true if the given task exists flase otherwise
    pub fn put_result(&self, task_id: &MsgId, result: T::Result) -> Result<(), TaskManagerError> {
        let Some(mut entry) = self.tasks.get_mut(task_id) else {
            return Err(TaskManagerError::NotFound);
        };
        if !entry.get_to().contains(result.get_from()) {
            return Err(TaskManagerError::Unauthorized)
        }
        let n = entry.msg.get_results().len();
        entry.msg.push_result(result);
        // We dont care if noone is listening
        _ = self.new_results
            .get(task_id)
            .expect("This task id must be present because it is present at the start of the function")
            .send(n);
        Ok(())
    }

    pub fn post_task(&self, task: MsgSigned<T>) -> Result<(), TaskManagerError> {
        let id = task.wait_id();
        if self.tasks.contains_key(&id) {
            return Err(TaskManagerError::Conflict);
        }
        let max_recievers = task.get_to().len();
        self.tasks.insert(id.clone(), task);
        let (results_sender, _) = broadcast::channel(max_recievers);
        self.new_results.insert(id.clone(), results_sender);
        // We dont care if noone is listening
        _ = self.new_tasks.send(id);
        Ok(())
    }
}

pub enum TaskManagerError {
    NotFound,
    Conflict,
    Unauthorized,
    Gone,
    BroadcastBufferOverflow
}

impl From<TaskManagerError> for StatusCode {
    fn from(value: TaskManagerError) -> Self {
        match value {
            TaskManagerError::NotFound => StatusCode::NOT_FOUND,
            TaskManagerError::Conflict => StatusCode::CONFLICT,
            TaskManagerError::BroadcastBufferOverflow => StatusCode::INTERNAL_SERVER_ERROR,
            TaskManagerError::Unauthorized => StatusCode::UNAUTHORIZED,
            TaskManagerError::Gone => StatusCode::GONE,
        }
    }
}
