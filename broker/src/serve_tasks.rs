use std::{
    convert::Infallible, net::SocketAddr,
    sync::Arc
};

use axum::{
    extract::ConnectInfo,
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{sse::Event, IntoResponse, Response, Sse},
    routing::{get, put},
    Json, Router,
};
use beam_lib::AppOrProxyId;
use futures::{Stream, StreamExt};
use hyper::HeaderMap;
use serde::{Deserialize, Serialize};
use beam_lib::WorkStatus;
use shared::{
    EncryptedMsgTaskRequest, EncryptedMsgTaskResult, HowLongToBlock, Msg, MsgEmpty,
    MsgId, MsgSigned
};
use tracing::{debug, trace};

use crate::task_manager::{TaskManager, Task, HasStatus, TaskWithResults};

#[derive(Clone)]
struct TasksState {
    task_manager: Arc<TaskManager<EncryptedMsgTaskRequest>>
}

pub(crate) fn router() -> Router {
    let state = TasksState::default();
    Router::new()
        .route("/v1/tasks", get(get_tasks).post(post_task))
        .route("/v1/tasks/:task_id/results", get(get_results_for_task))
        .route("/v1/tasks/:task_id/results/:app_id", put(put_result))
        .with_state(state)
}

impl Default for TasksState {
    fn default() -> Self {
        TasksState {
            task_manager: TaskManager::new()
        }
    }
}

async fn get_results_for_task(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<TasksState>,
    block: HowLongToBlock,
    Path(task_id): Path<MsgId>,
    headers: HeaderMap,
    msg: MsgSigned<MsgEmpty>,
) -> Response {
    let found = &headers
        .get(header::ACCEPT)
        .unwrap_or(&HeaderValue::from_static(""))
        .to_str()
        .unwrap_or_default()
        .split(',')
        .map(|part| part.trim())
        .find(|part| *part == "text/event-stream")
        .is_some();

    if *found {
        get_results_for_task_stream(addr, state, block, task_id, msg)
            .await
            .into_response()
    } else {
        get_results_for_task_nostream(addr, state, block, task_id, msg)
            .await
            .into_response()
    }
}

// GET /v1/tasks/:task_id/results
async fn get_results_for_task_nostream(
    addr: SocketAddr,
    state: TasksState,
    block: HowLongToBlock,
    task_id: MsgId,
    msg: MsgSigned<MsgEmpty>,
) -> Result<(StatusCode, Json<impl Serialize>), StatusCode> {
    debug!(
        "get_results_for_task(task={}) called by {} with IP {addr}, wait={:?}",
        task_id.to_string(),
        msg.get_from(),
        block
    );
    let task = state.task_manager
        .get(task_id)
        .await?;
    if msg.get_from() != task.get_from() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    
    let tasks = task.get_results(block, |_| true).await;
    let status = if block.wait_count.is_some_and(|wc| tasks.len() < wc.into()) {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    Ok((status, Json(tasks)))
}

// GET /v1/tasks/:task_id/results/stream
async fn get_results_for_task_stream(
    addr: SocketAddr,
    state: TasksState,
    block: HowLongToBlock,
    task_id: MsgId,
    msg: MsgSigned<MsgEmpty>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, StatusCode> {
    debug!(
        "get_results_for_task_stream(task={}) called by {} with IP {addr}, wait={:?}",
        task_id.to_string(),
        msg.get_from(),
        block
    );
    let from = msg.get_from().clone();
    if &from != state.task_manager.get(task_id).await?.get_from() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let task = state.task_manager
        .get(task_id)
        .await?;
    let stream = task
        .stream_result_events(block, |_| true)
        .map(Ok);

    Ok(Sse::new(stream))
}

#[derive(Deserialize)]
struct TaskFilter {
    from: Option<AppOrProxyId>,
    to: Option<AppOrProxyId>,
    filter: Option<FilterParam>,
}

impl TaskFilter {
    async fn filter<T>(
        &self,
        task_with_results: impl AsRef<TaskWithResults<T>>,
        caller_id: &AppOrProxyId,
    ) -> bool
    where
        T: Task + Msg,
        T::Result: HasStatus + Send + Sync + 'static,
    {
        let task_with_results = task_with_results.as_ref();
        let is_for_me = task_with_results.get_to().contains(&caller_id);
        let from_matches = match &self.from {
            Some(from) => task_with_results.get_from() == from,
            None => true,
        };
        // Not sure if we want to keep this.
        // Previously this was mainly used internally to verify `is_for_me` in a hard to follow way
        // but could be set externally as well which I am really not sure what that did.
        // So I implemented this which is what I would expect a `to` filter to check.
        // With this you could filter a task that is for you and some other service
        // that is not you which is information you would otherwise only obtain on task reception.
        let to_matches = match &self.to {
            Some(to) => task_with_results.get_to().contains(&to),
            None => true,
        };
        let filter_matches = match self.filter {
            Some(FilterParam::Todo) => {
                if let Some(result) = task_with_results.get_result(&caller_id).await {
                    ![WorkStatus::Succeeded, WorkStatus::PermFailed].contains(&result.get_status())
                } else {
                    true
                }
            }
            None => true,
        };
        is_for_me && from_matches && to_matches && filter_matches
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum FilterParam {
    Todo,
}

/// GET /v1/tasks
/// Will retrieve tasks that are at least FROM or TO the supplied parameters.
async fn get_tasks(
    block: HowLongToBlock,
    Query(ref taskfilter): Query<TaskFilter>,
    State(state): State<TasksState>,
    msg: MsgSigned<MsgEmpty>,
) -> (StatusCode, Json<impl Serialize>) {
    let from = &msg.msg.from;
    let tasks: Vec<_> = state.task_manager
        // I really want to get rid of the arc clone but I think its only possible when we get async closures with async fn traits
        .stream_tasks(block, move |t| taskfilter.filter(Arc::clone(t), from))
        .collect()
        .await;
    let status = if block.wait_count.is_some_and(|wc| tasks.len() < wc.into()) {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    (status, Json(tasks))
}

// POST /v1/tasks
async fn post_task(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<TasksState>,
    msg: MsgSigned<EncryptedMsgTaskRequest>,
) -> Result<(StatusCode, impl IntoResponse), StatusCode> {
    trace!(
        "Client {} with IP {addr} is creating task {:?}",
        msg.msg.from, msg
    );
    let id = msg.msg.id;
    state.task_manager.post_task(msg).await?;
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, format!("/v1/tasks/{}", id))],
    ))
}

// PUT /v1/tasks/:task_id/results/:app_id
async fn put_result(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path((task_id, app_id)): Path<(MsgId, AppOrProxyId)>,
    State(state): State<TasksState>,
    result: MsgSigned<EncryptedMsgTaskResult>,
) -> Result<StatusCode, (StatusCode, &'static str)> {
    trace!("Called: Task {:?}, {:?} by {addr}", task_id, result);
    if task_id != result.msg.task {
        return Err((
            StatusCode::BAD_REQUEST,
            "Task IDs supplied in path and payload do not match.",
        ));
    }
    let worker_id = result.msg.from.clone();
    if app_id != worker_id {
        return Err((
            StatusCode::BAD_REQUEST,
            "AppID supplied in URL and signed message do not match.",
        ));
    }


    let status = if state.task_manager.put_result(task_id, result).await? {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CREATED
    };
    Ok(status)
}
