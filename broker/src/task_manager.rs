use std::{
    collections::HashSet,
    ops::Deref,
    sync::Arc,
    time::{Duration, SystemTime}
};

use axum::response::sse::Event;
use beam_lib::{AppOrProxyId, MsgId, WorkStatus};
use futures::Stream;
use hyper::StatusCode;
use moka::{future::Cache, Expiry};
use serde::Serialize;
use shared::{
    sse_event::SseEventType, HowLongToBlock, Msg, MsgSigned, MsgState, MsgTaskRequest,
    MsgTaskResult,
};
use tokio::{sync::broadcast, time::Instant};
use tokio_stream::wrappers::BroadcastStream;
use tracing::{error, warn};

/// In Rust edition 2024 this will no longer be needed
/// This is a helper trait to express that a RPIT captures a lifetime
/// Using `impl Trait + 'a` is not what you want to return its `impl Trait + Captures<&'a ()>`
/// See this [talk](https://www.youtube.com/watch?v=CWiz_RtA1Hw) from Jon Gjengset
pub trait Captures<U> {}
impl<T: ?Sized, U> Captures<U> for T {}

pub trait Task {
    type Result;

    fn get_expire_time(&self) -> &SystemTime;
    fn get_id(&self) -> MsgId;
}

pub trait HasStatus {
    fn get_status(&self) -> WorkStatus;
}

impl<State: MsgState> Task for MsgTaskRequest<State> {
    type Result = MsgSigned<MsgTaskResult<State>>;

    fn get_expire_time(&self) -> &SystemTime {
        &self.expire
    }

    fn get_id(&self) -> MsgId {
        self.id
    }
}

#[cfg(feature = "sockets")]
impl<State: MsgState> Task for shared::MsgSocketRequest<State> {
    type Result = ();

    fn get_expire_time(&self) -> &SystemTime {
        &self.expire
    }

    fn get_id(&self) -> MsgId {
        self.id
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

struct TaskExpirey;

impl<K, T: Task + Msg> Expiry<K, (Instant, Arc<TaskWithResults<T>>)> for TaskExpirey {
    /// TODO: use the created at from the fn and store an instance for the expire property of task
    fn expire_after_create(
        &self,
        _: &K,
        value: &(Instant, Arc<TaskWithResults<T>>),
        _: std::time::Instant,
    ) -> Option<Duration> {
        value
            .1
            .task
            .msg
            .get_expire_time()
            .duration_since(SystemTime::now())
            .ok()
    }
}

pub struct TaskWithResults<T: Task + Msg> {
    task: MsgSigned<T>,
    results: Cache<AppOrProxyId, (Instant, Arc<T::Result>)>,
    new_results: broadcast::Sender<Arc<T::Result>>,
}

impl<T> TaskWithResults<T>
where
    T: Task + Msg,
    T::Result: Sync + Send + 'static,
{
    fn new(task: MsgSigned<T>) -> Self {
        let max_results = task.get_to().len();
        Self {
            task,
            results: Cache::builder().initial_capacity(max_results).build(),
            new_results: broadcast::channel(1.max(max_results)).0,
        }
    }

    pub async fn get_result(&self, from: &AppOrProxyId) -> Option<Arc<T::Result>> where T::Result: 'static {
        self.results.get(from).await.map(|v| v.1)
    }

    fn stream_results_inner(
        &self,
        max_elements: usize,
        filter: impl Fn(&T::Result) -> bool,
    ) -> impl Stream<Item = Arc<T::Result>> + Captures<&'_ ()>
    where
        T::Result: HasStatus + 'static,
    {
        use tokio_stream::StreamExt;
        let new_results = self.new_results.subscribe();
        let ts = Instant::now();
        let initial_results = self
            .results
            .into_iter()
            .filter_map(move |(_, (insertion_time, msg))| (insertion_time <= ts).then_some(msg));
        let stream = tokio_stream::iter(initial_results)
            .chain(
                BroadcastStream::new(new_results).filter_map(move |new_result| match new_result {
                    Ok(result) => Some(result),
                    Err(e) => {
                        warn!("new_results channel lagged: {e}");
                        None
                    }
                }),
            )
            .filter(move |res| filter(&res));
        let mut num_of_results = 0;
        async_stream::stream! {
            for await res in stream {
                let status = res.get_status();
                yield res;
                if status != WorkStatus::Claimed {
                    num_of_results += 1;
                    if num_of_results >= max_elements {
                        break
                    }
                }
            }
        }
    }

    pub async fn get_results(
        &self,
        block: HowLongToBlock,
        filter: impl Fn(&T::Result) -> bool,
    ) -> Vec<Arc<T::Result>>
    where
        T::Result: HasStatus + Msg + 'static,
    {
        use futures::StreamExt;
        struct HashMsg<M: Msg>(Arc<M>);
        // Dedupe stream by the proxy id only keeping the most recent result
        impl<M: Msg> std::hash::Hash for HashMsg<M> {
            fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                self.0.get_from().proxy_id().hash(state)
            }
        }
        impl<M: Msg> PartialEq for HashMsg<M> {
            fn eq(&self, other: &Self) -> bool {
                self.0.get_from().proxy_id() == other.0.get_from().proxy_id()
            }
        }
        impl<M: Msg> Eq for HashMsg<M> {}

        let (max_elements, wait_until) = decide_blocking_conditions(&block);
        let mut s = std::pin::pin!(self.stream_results_inner(max_elements, filter).take_until(tokio::time::sleep_until(wait_until)));
        let mut seen = HashSet::with_capacity(self.get_to().len());
        while let Some(t) = s.next().await {
            seen.insert(HashMsg(t));
        }
        seen.into_iter().map(|v| v.0).collect()
    }

    pub fn stream_result_events(
        self: Arc<Self>,
        block: HowLongToBlock,
        filter: impl Fn(&T::Result) -> bool + 'static,
    ) -> impl Stream<Item = Event> + 'static
    where
        T::Result: HasStatus + Serialize + 'static,
        T: 'static
    {
        use futures::StreamExt;

        async_stream::stream! {
            let (max_results, wait_until) = decide_blocking_conditions(&block);
            if block.wait_time.is_none() && block.wait_count.is_none() {
                for await v in self.stream_results_inner(max_results, filter).take_until(tokio::time::sleep_until(wait_until)) {
                    yield to_event(v.as_ref(), SseEventType::NewResult);
                }
                return;
            };
            let expired = &mut false;
            let st = self.stream_results_inner(max_results, filter).take_until(async {
                tokio::time::sleep_until(wait_until).await;
                *expired = true;
            });
            for await res in st {
                yield to_event(res.as_ref(), SseEventType::NewResult);
            }
            if *expired {
                yield to_event((), SseEventType::WaitExpired);
            }
        }
    }
}

impl<T> Deref for TaskWithResults<T>
where
    T: Task + Msg,
{
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.task.msg
    }
}

impl<T> Serialize for TaskWithResults<T>
where
    T: Task + Msg + Serialize
{
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.task.serialize(serializer)
    }
}

pub struct TaskManager<T: Task + Msg> {
    tasks: Cache<MsgId, (Instant, Arc<TaskWithResults<T>>)>,
    new_tasks: broadcast::Sender<Arc<TaskWithResults<T>>>,
}

impl<T> TaskManager<T>
where
    T: Task + Msg + Send + Sync + 'static,
    T::Result: Send + Sync,
{
    pub fn new() -> Arc<Self> {
        let (new_tasks, _) = broadcast::channel(256);
        Arc::new(Self {
            tasks: Cache::builder().expire_after(TaskExpirey).build(),
            new_tasks,
        })
    }
}

impl<T> TaskManager<T>
where
    T: Task + Msg + 'static,
    T::Result: Send + Sync,
    TaskWithResults<T>: Send + Sync
{
    pub async fn get(&self, task_id: MsgId) -> Result<Arc<TaskWithResults<T>>, TaskManagerError> {
        self.tasks
            .get(&task_id)
            .await
            .map(|v| v.1)
            .ok_or(TaskManagerError::NotFound)
    }

    pub async fn remove(&self, task_id: MsgId) {
        self.tasks.invalidate(&task_id).await
    }

    /// ## Note:
    /// This function may yield tasks that expired while waiting on new ones
    pub fn stream_tasks(
        &self,
        block: HowLongToBlock,
    ) -> impl Stream<Item = Arc<TaskWithResults<T>>> + Captures<&'_ ()>
    {
        let (max_elements, wait_until) = decide_blocking_conditions(&block);
        let new_tasks = self.new_tasks.subscribe();
        let ts = Instant::now();
        let initial_tasks = self
            .tasks
            .into_iter()
            .filter_map(move |(_, (insertion_time, msg))| (insertion_time <= ts).then_some(msg));
        let new_task_stream = {
            use tokio_stream::StreamExt;
            BroadcastStream::new(new_tasks)
                .filter_map(|new_task| match new_task {
                    Ok(task) => Some(task),
                    Err(e) => {
                        warn!("new_tasks channel lagged: {e}");
                        None
                    }
                })
        };
        {
            use futures::StreamExt;
            tokio_stream::iter(initial_tasks)
                .chain(new_task_stream)
                .take(max_elements)
                .take_until(tokio::time::sleep_until(wait_until))
        }
    }

    pub async fn post_task(&self, task: MsgSigned<T>) -> Result<(), TaskManagerError> {
        let entry = self
            .tasks
            .entry(task.msg.get_id())
            .or_insert_with(async {
                let task_with_results = Arc::new(TaskWithResults::new(task));
                // We dont care if noone is listening
                _ = self.new_tasks.send(task_with_results.clone());
                (Instant::now(), task_with_results)
            })
            .await;
        if !entry.is_fresh() {
            warn!("Conflict: Task with id {} already existed", entry.key());
            return Err(TaskManagerError::Conflict);
        }
        Ok(())
    }
}

fn decide_blocking_conditions(block: &HowLongToBlock) -> (usize, Instant) {
    match (block.wait_count, block.wait_time) {
        // Only wait a very short time so that all tasks that are ready get yielded
        (None, None) => (usize::MAX, Instant::now() + Duration::from_millis(100)),
        // Wait for as long as specified regardless of the number of elements
        (None, Some(wait_time)) => (usize::MAX, Instant::now() + wait_time),
        // Wait for n elements or timeout after 1h
        (Some(wait_count), None) => (
            wait_count as usize,
            Instant::now() + Duration::from_secs(60 * 60),
        ),
        // Stop waiting after either some time or some number of elements
        (Some(wait_count), Some(wait_time)) => (wait_count as usize, Instant::now() + wait_time),
    }
}

impl<T> TaskManager<T>
where
    T: Task + Msg + 'static,
    T::Result: Msg + Send + Sync,
    TaskWithResults<T>: Send + Sync
{
    /// This will push the result to the given task by its id.
    /// Returns true if the given result was an update to an existing result
    pub async fn put_result(&self, task_id: MsgId, result: T::Result) -> Result<bool, TaskManagerError> {
        let task = self.get(task_id).await?;
        if !task.get_to().contains(result.get_from()) {
            return Err(TaskManagerError::Unauthorized);
        }
        let sender = result.get_from().clone();
        let result = Arc::new(result);
        // We dont care if noone is listening
        _ = task.new_results.send(result.clone());
        let mut is_updated = false;
        task
            .results
            .entry(sender)
            // I did not find another way to check if the result was actually present
            .or_insert_with_if(std::future::ready((Instant::now(), result)), |_| {
                is_updated = true;
                true
            }).await;
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
    Event::default()
        .event(event_type)
        .json_data(json)
        .unwrap_or_else(|e| {
            error!("Unable to serialize message: {e}");
            Event::default()
                .event(SseEventType::Error)
                .data("Internal error: Unable to serialize message.")
        })
}

#[cfg(test)]
mod test {
    use super::*;
    use beam_lib::AppId;
    use futures::StreamExt;
    use shared::Plain;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_all() {
        const N: u32 = 1000;
        const CONCURRENT_TASKS: u8 = 2;
        futures::future::join_all((0..N).map(|_| async move {
            let tm = TaskManager::<MsgTaskRequest>::new();
            let a = futures::future::join_all((0..CONCURRENT_TASKS).map(|_| tokio::spawn(add_task(tm.clone()))));
            assert!(tokio::try_join!(
                tokio::spawn(assert_n_tasks(CONCURRENT_TASKS, tm.clone())),
                async {
                    a.await.into_iter().collect::<Result<Vec<_>, _>>()
                }
            ).is_ok());
        })).await;
    }

    async fn assert_n_tasks(n: u8, tm: Arc<TaskManager<MsgTaskRequest>>) {
        let s = tm.stream_tasks(HowLongToBlock { wait_time: Some(Duration::from_millis(250)), wait_count: None });
        futures::pin_mut!(s);
        for _ in 0..n {
            assert!(s.next().await.is_some(), "Had no task");
        }
        assert!(s.next().await.is_none(), "Had more tasks");
    }

    async fn add_task(tm: Arc<TaskManager<MsgTaskRequest>>) {
        tm.post_task(MsgSigned { msg: MsgTaskRequest {
            id: MsgId::new(),
            from: AppId::new_unchecked("").into(),
            to: vec![],
            body: Plain::from(""),
            expire: SystemTime::now() + Duration::from_secs(60),
            failure_strategy: beam_lib::FailureStrategy::Discard,
            metadata: serde_json::Value::Null,
        }, jwt: String::new() }).await.unwrap()
    }
}
