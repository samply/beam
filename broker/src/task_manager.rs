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
use shared::{
    beam_id::AppOrProxyId, HasWaitId, HowLongToBlock, Msg, MsgEmpty, MsgId, MsgSigned,
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
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Self::EXPIRE_CHECK_INTERVAL).await;
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
        let max_elements = block.wait_count.unwrap_or(u16::MAX) as usize;
        let wait_until = Instant::now() + block.wait_time.unwrap_or(Duration::from_secs(600));
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
        let (results_sender, _) = broadcast::channel(max_receivers);
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
    T::Result: Msg,
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
            .filter(|result| filter(result))
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

        // Somehow mapping this task to its results creates lifetime issues that I failed to solve.
        // So the caller needs to get the results himself which is not to bad I guess.
        // FIXME: Return results here
        self.get(task_id).map_err(|_| TaskManagerError::Gone)
    }

    pub fn stream_results<'a>(
        &'a self,
        task_id: &'a MsgId,
        block: &'a HowLongToBlock,
        filter: impl Fn(&T::Result) -> bool + 'a
    ) -> impl Stream<Item = Result<Event, Infallible>> + 'a
        where T::Result: Serialize
    {
        async_stream::stream! {
            let Ok(task) = self.get(task_id) else {
                yield Ok(to_event("Did not find task", SseEventType::Error));
                return;
            };
            let (max_elements, wait_until) = decide_blocking_conditions(block);
            let ready_results = task.msg
                .get_results()
                .values()
                .filter(|result| filter(result));
            let mut num_of_results = 0;
            for res in ready_results {
                yield Ok(to_event(res, SseEventType::NewResult));
                num_of_results += 1;
                // Only break when wait_count was actually set otherwise we want all the tasks that are present
                if num_of_results >= max_elements && max_elements!=0 {
                    break;
                }
            }
            let mut new_results = self
                .new_results
                .get(task_id)
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
                                if let Ok(task) = self.get(task_id) {
                                    let new_result = &task.msg.get_results()[&key];
                                    if filter(new_result) {
                                        num_of_results += 1;
                                        yield Ok(to_event(new_result, SseEventType::NewResult));
                                    };
                                } else {
                                    yield Ok(to_event(json!({"task_id": task_id}), SseEventType::DeletedTask));
                                }
                            },
                            Err(e) => {
                                warn!("new_results channel lagged: {e}");
                                yield Ok(to_event("Internal server error", SseEventType::Error));
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
