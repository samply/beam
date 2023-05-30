use std::{
    borrow::Cow,
    ops::Deref,
    time::{Duration, SystemTime}, collections::HashMap,
};

use axum::{response::IntoResponse, Json};
use dashmap::{
    mapref::{multiple::RefMulti, one::Ref},
    DashMap,
};
use hyper::StatusCode;
use once_cell::sync::Lazy;
use shared::{
    beam_id::AppOrProxyId, HasWaitId, HowLongToBlock, Msg, MsgEmpty, MsgId, MsgSigned,
    MsgSocketRequest, MsgState, MsgTaskRequest, MsgTaskResult,
};
use tokio::{sync::broadcast, time::Instant};
use tracing::warn;

pub trait Task {
    type Result;

    fn get_results(&self) -> &HashMap<AppOrProxyId, Self::Result>;
    /// Returns true if the value as been updated and false if it was a result from a new app
    fn insert_result(&mut self, result: Self::Result) -> bool;
    fn is_expired(&self) -> bool;
}

impl<State: MsgState> Task for MsgTaskRequest<State> {
    type Result = MsgSigned<MsgTaskResult<State>>;

    fn insert_result(&mut self, result: Self::Result) -> bool {
        self.results.insert(result.get_from().clone(), result).is_some()
    }

    fn get_results(&self) -> &HashMap<AppOrProxyId, Self::Result> {
        &self.results
    }

    fn is_expired(&self) -> bool {
        self.expire < SystemTime::now()
    }
}

static EMPTY_MAP: Lazy<HashMap<AppOrProxyId, ()>> = Lazy::new(|| {
    HashMap::with_capacity(0)
});

impl<State: MsgState> Task for MsgSocketRequest<State> {
    type Result = ();

    fn get_results(&self) -> &HashMap<AppOrProxyId, Self::Result> {
        &EMPTY_MAP
    }

    fn insert_result(&mut self, _result: Self::Result) -> bool { false }

    fn is_expired(&self) -> bool {
        self.expire < SystemTime::now()
    }
}

pub struct TaskManager<T: HasWaitId<MsgId> + Task + Msg> {
    tasks: DashMap<MsgId, MsgSigned<T>>,
    new_tasks: broadcast::Sender<MsgId>,
    deleted_tasks: broadcast::Sender<MsgId>,
    /// Send the index at which the new result for the given Task was inserted
    new_results: DashMap<MsgId, broadcast::Sender<AppOrProxyId>>,
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
            new_results: Default::default(),
        }
    }

    pub fn get(&self, task_id: &MsgId) -> Result<impl Deref<Target = MsgSigned<T>> + '_, TaskManagerError> {
        self.tasks.get(task_id).ok_or(TaskManagerError::NotFound)
    }

    pub fn get_tasks_by(&self, filter: impl Fn(&T) -> bool) -> impl Iterator<Item = impl Deref<Target = MsgSigned<T>> + '_> {
        self.tasks
            .iter()
            .filter(move |entry| filter(&entry.msg))
            .filter(|entry| !entry.msg.is_expired())
    }

    // Once async iterators are stabelized this should be one
    pub async fn wait_for_tasks(
        &self,
        block: &HowLongToBlock,
        filter: impl Fn(&T) -> bool,
    ) -> Result<impl Iterator<Item = impl Deref<Target = MsgSigned<T>> + '_>, TaskManagerError>
    {
        let max_elements = block.wait_count.unwrap_or(u16::MAX) as usize;
        let wait_until = Instant::now() + block.wait_time.unwrap_or(Duration::from_secs(600));
        let mut new_tasks = self.new_tasks.subscribe();
        let mut deleted_tasks = self.deleted_tasks.subscribe();

        let mut num_of_tasks = self.get_tasks_by(&filter).count();
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

impl<T: HasWaitId<MsgId> + Task + Msg> TaskManager<T>
where
    T::Result: Msg,
{
    /// This does not check if the requester was the creator of the Task
    pub async fn wait_for_results(
        &self,
        task_id: &MsgId,
        block: &HowLongToBlock,
        filter: impl Fn(&T::Result) -> bool,
    ) -> Result<impl Deref<Target = MsgSigned<T>> + '_, TaskManagerError> {
        let max_elements = block.wait_count.unwrap_or(u16::MAX) as usize;
        let wait_until = Instant::now() + block.wait_time.unwrap_or(Duration::from_secs(600));

        let mut num_of_results = self
            .get(task_id)?
            .msg
            .get_results()
            .values()
            .filter(|result| filter(result))
            .count();
        let mut new_results = self
            .new_results
            .get(task_id)
            .ok_or(TaskManagerError::NotFound)?
            .subscribe();
        while num_of_results < max_elements && Instant::now() < wait_until {
            tokio::select! {
                _ = tokio::time::sleep_until(wait_until) => {
                    break;
                },
                result = new_results.recv() => {
                    match result {
                        Ok(key) => {
                            if let Ok(task) = self.get(task_id) {
                                if filter(&task.msg.get_results()[&key]) {
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

    /// This will push the result to the given task by its id
    /// Returns true if the given task exists flase otherwise
    pub fn put_result(&self, task_id: &MsgId, result: T::Result) -> Result<(), TaskManagerError> {
        let Some(mut entry) = self.tasks.get_mut(task_id) else {
            return Err(TaskManagerError::NotFound);
        };
        if !entry.get_to().contains(result.get_from()) {
            return Err(TaskManagerError::Unauthorized);
        }
        let sender = result.get_from().clone();
        entry.msg.insert_result(result);
        // We dont care if noone is listening
        _ = self
            .new_results
            .get(task_id)
            .expect(
                "This task id must be present because it is present at the start of the function",
            )
            .send(sender);
        Ok(())
    }
}

pub enum TaskManagerError {
    NotFound,
    Conflict,
    Unauthorized,
    Gone,
    BroadcastBufferOverflow,
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
