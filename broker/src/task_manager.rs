use std::{
    borrow::Cow,
    ops::Deref,
    time::{Duration, SystemTime}, collections::HashMap, sync::Arc, convert::Infallible,
};

use axum::{response::{IntoResponse, sse::Event, Sse}, Json};
use dashmap::DashMap;
use futures_core::Stream;
use hyper::StatusCode;
use once_cell::sync::Lazy;
use serde::Serialize;
use serde_json::json;
use beam_lib::{AppOrProxyId, MsgEmpty, MsgId, WorkStatus};
use shared::{
    HasWaitId, HowLongToBlock, Msg, MsgSigned,
    MsgState, MsgTaskRequest, MsgTaskResult, sse_event::SseEventType,
};
use tokio::{sync::broadcast, time::Instant};
use tracing::{warn, error};

pub trait Task {
    type Result;

    fn get_results(&self) -> &HashMap<AppOrProxyId, Self::Result>;
    /// Returns true if the value as been updated and false if it was a result from a new app
    fn insert_result(&mut self, result: Self::Result) -> bool;
    fn is_expired(&self) -> bool;
}

pub trait HasStatus {
    fn get_status(&self) -> WorkStatus;
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

#[cfg(feature = "sockets")]
impl<State: MsgState> Task for shared::MsgSocketRequest<State> {
    type Result = ();

    fn get_results(&self) -> &HashMap<AppOrProxyId, Self::Result> {
        &EMPTY_MAP
    }

    fn insert_result(&mut self, _result: Self::Result) -> bool { false }

    fn is_expired(&self) -> bool {
        self.expire < SystemTime::now()
    }
}

impl<T: MsgState> HasStatus for MsgTaskResult<T> {
    fn get_status(&self) -> WorkStatus {
        self.status
    }
}

impl<T: HasStatus + Msg> HasStatus for MsgSigned<T> {
    fn get_status(&self) -> WorkStatus {
        self.msg.get_status()
    }
}

pub struct TaskManager<T: HasWaitId<MsgId> + Task + Msg> {
    tasks: DashMap<MsgId, MsgSigned<T>>,
    new_tasks: broadcast::Sender<MsgId>,
    /// Send the index at which the new result for the given Task was inserted
    new_results: DashMap<MsgId, broadcast::Sender<AppOrProxyId>>,
}

impl<T: HasWaitId<MsgId> + Task + Msg + Send + Sync + 'static> TaskManager<T> {
    const EXPIRE_CHECK_INTERVAL: Duration = Duration::from_secs(5 * 60);

    pub fn new() -> Arc<Self> {
        let (new_tasks, _) = broadcast::channel(256);
        let task_manager = Arc::new(Self {
            tasks: Default::default(),
            new_tasks,
            new_results: Default::default(),
        });
        let tm = Arc::clone(&task_manager);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Self::EXPIRE_CHECK_INTERVAL);
                tm.tasks.retain(|_, task| if task.msg.is_expired() {
                    tm.new_results.remove(&task.msg.wait_id());
                    false
                } else {
                    true
                });
                // If the memory footprint of the Dashmap will get too large we might need to consider calling DashMap::shrink_to_fit or find a better solution as
                // this would need to lock the whole map making it inaccessible until everything is reallocated
            }
        });

        task_manager
    }
}

impl<T: HasWaitId<MsgId> + Task + Msg> TaskManager<T> {

    pub fn get(&self, task_id: &MsgId) -> Result<impl Deref<Target = MsgSigned<T>> + '_, TaskManagerError> {
        self.tasks.get(task_id).ok_or(TaskManagerError::NotFound)
    }

    pub fn remove(&self, task_id: &MsgId) -> Result<MsgSigned<T>, TaskManagerError> {
        self.tasks.remove(task_id).ok_or(TaskManagerError::NotFound).map(|v| v.1)
    }

    pub fn get_tasks_by(&self, filter: impl Fn(&T) -> bool) -> impl Iterator<Item = impl Deref<Target = MsgSigned<T>> + '_> {
        self.tasks
            .iter()
            .filter(move |entry| filter(&entry.msg))
            .filter(|entry| !entry.msg.is_expired())
    }

    // Once async iterators are stabilized this should be one
    /// ## Note:
    /// This function may yield less tasks than `block.wait_count` if tasks expired while waiting on new ones
    pub async fn wait_for_tasks(
        &self,
        block: &HowLongToBlock,
        filter: impl Fn(&T) -> bool,
    ) -> Result<impl Iterator<Item = impl Deref<Target = MsgSigned<T>> + '_>, TaskManagerError>
    {
        let (max_elements, wait_until) = decide_blocking_conditions(block);
        let mut new_tasks = self.new_tasks.subscribe();

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
            }
        }
        Ok(self.get_tasks_by(filter))
    }

    pub fn post_task(&self, task: MsgSigned<T>) -> Result<(), TaskManagerError> {
        let id = task.wait_id();
        if let Some(task) = self.tasks.get(&id) {
            // We only have a conflict if the conflicting task has not yet expired
            if !task.msg.is_expired() {
                return Err(TaskManagerError::Conflict);
            }
        }
        let max_receivers = task.get_to().len();
        self.tasks.insert(id.clone(), task);
        let (results_sender, _) = broadcast::channel(1.max(max_receivers));
        self.new_results.insert(id.clone(), results_sender);
        // We dont care if noone is listening
        _ = self.new_tasks.send(id);
        Ok(())
    }
}

fn decide_blocking_conditions(block: &HowLongToBlock) -> (usize, Instant) {
    match (block.wait_count, block.wait_time) {
        // Dont wait
        (None, None) => (0, Instant::now()),
        // Wait for as long as specified regardless of the number of elements
        (None, Some(wait_time)) => (usize::MAX, Instant::now() + wait_time),
        // Wait for n elements or timeout after 1h
        (Some(wait_count), None) => (wait_count as usize, Instant::now() + Duration::from_secs(60 * 60)),
        // Stop waiting after either some time or some number of elements
        (Some(wait_count), Some(wait_time)) => (wait_count as usize, Instant::now() + wait_time),
    }
}

impl<T: HasWaitId<MsgId> + Task + Msg> TaskManager<T>
where
    T::Result: Msg + HasStatus,
{
    /// This does not check if the requester was the creator of the Task
    pub async fn wait_for_results(
        &self,
        task_id: &MsgId,
        block: &HowLongToBlock,
        filter: impl Fn(&T::Result) -> bool,
    ) -> Result<impl Deref<Target = MsgSigned<T>> + '_, TaskManagerError> {
        let (max_elements, wait_until) = decide_blocking_conditions(block);
        let mut num_of_results = self
            .get(task_id)?
            .msg
            .get_results()
            .values()
            .filter(|result| filter(result) && result.get_status() != WorkStatus::Claimed)
            .count();
        let mut new_results = self
            .new_results
            .get(task_id)
            .expect("Found task but no corresponding results channel")
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
                                let result = &task.msg.get_results()[&key];
                                if filter(result) && result.get_status() != WorkStatus::Claimed {
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

        // Somehow mapping this task to its results creates lifetime issues that I failed to solve.
        // So the caller needs to get the results himself which is not to bad I guess.
        // FIXME: Return results here
        self.get(task_id).map_err(|_| TaskManagerError::Gone)
    }

    pub fn stream_results(
        self: Arc<Self>,
        task_id: MsgId,
        block: HowLongToBlock,
        filter: impl Fn(&T::Result) -> bool + 'static + Send + Sync
    ) -> impl Stream<Item = Result<Event, Infallible>> + 'static + Send
        where
            T::Result: Serialize + Sync + Send,
            T: Send + Sync + 'static
    {
        async_stream::stream! {
            let Ok(task) = self.get(&task_id) else {
                yield Ok(to_event("Did not find task", SseEventType::Error));
                return;
            };
            let (max_elements, wait_until) = decide_blocking_conditions(&block);
            let ready_results = task.msg
                .get_results()
                .values()
                .filter(|result| filter(result));
            let mut num_of_results = 0;
            let mut events = Vec::with_capacity(task.msg.get_results().len());
            for res in ready_results {
                if res.get_status() != WorkStatus::Claimed {
                    num_of_results += 1;
                }
                events.push(to_event(res, SseEventType::NewResult));
                // Only break when wait_count was actually set otherwise we want all the tasks that are present
                if num_of_results >= max_elements && max_elements != 0 {
                    break;
                }
            }
            // Drop lock before doing async stuff
            drop(task);
            for event in events {
                yield Ok(event);
            }
            let mut new_results = self
                .new_results
                .get(&task_id)
                .expect("Found task but no corresponding results channel")
                .subscribe();
            while num_of_results < max_elements && Instant::now() < wait_until {
                tokio::select! {
                    _ = tokio::time::sleep_until(wait_until) => {
                        yield Ok(to_event((), SseEventType::WaitExpired));
                        break;
                    },
                    result = new_results.recv() => {
                        match result {
                            Ok(key) => {
                                if let Ok(task) = self.get(&task_id) {
                                    let new_result = &task.msg.get_results()[&key];
                                    if filter(new_result) {
                                        if new_result.get_status() != WorkStatus::Claimed {
                                            num_of_results += 1;
                                        }
                                        let event = to_event(new_result, SseEventType::NewResult);
                                        drop(task);
                                        yield Ok(event);
                                    };
                                } else {
                                    yield Ok(to_event(json!({"task_id": task_id}), SseEventType::DeletedTask));
                                }
                            },
                            Err(broadcast::error::RecvError::Lagged(n)) => {
                                warn!("new_results channel lagged by: {n} results.");
                                yield Ok(to_event("Internal server error", SseEventType::Error));
                            },
                            Err(broadcast::error::RecvError::Closed) => {
                                yield Ok(to_event("Task expired", SseEventType::WaitExpired));
                                break;
                            }
                        }
                    },
                }
            }
        }
    }

    /// This will push the result to the given task by its id.
    /// Returns true if the given result was an update to an existing result
    pub fn put_result(&self, task_id: &MsgId, result: T::Result) -> Result<bool, TaskManagerError> {
        let Some(mut task) = self.tasks.get_mut(task_id) else {
            return Err(TaskManagerError::NotFound);
        };
        if !task.get_to().contains(result.get_from()) {
            return Err(TaskManagerError::Unauthorized);
        }
        let sender = result.get_from().clone();
        let is_updated = task.msg.insert_result(result);
        // We dont care if noone is listening
        _ = self
            .new_results
            .get(task_id)
            .expect(
                "This task id must be present because it is present at the start of the function",
            )
            .send(sender);
        Ok(is_updated)
    }
}

#[derive(Debug)]
pub enum TaskManagerError {
    NotFound,
    Conflict,
    Unauthorized,
    Gone,
    BroadcastBufferOverflow,
}

impl TaskManagerError {
    pub fn error_msg(&self) -> &'static str {
        match self {
            TaskManagerError::NotFound => "Task not found",
            TaskManagerError::Conflict => "Task already exists",
            TaskManagerError::Unauthorized => "Unauthorized to access this task",
            TaskManagerError::Gone => "Task expired while waiting on it",
            TaskManagerError::BroadcastBufferOverflow => "Internal server error",
        }
    }
}

impl From<TaskManagerError> for (StatusCode, &'static str) {
    fn from(value: TaskManagerError) -> Self {
        let err = value.error_msg();
        (StatusCode::from(value), err)
    }
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

fn to_event(json: impl Serialize, event_type: impl AsRef<str>) -> Event {
    Event::default().event(event_type).json_data(json).unwrap_or_else(|e| {
        error!("Unable to serialize message: {e}");
        Event::default()
            .event(SseEventType::Error)
            .data("Internal error: Unable to serialize message.")
    })
}
