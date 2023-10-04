use std::{
    collections::HashMap, convert::Infallible, fmt::Debug, mem::Discriminant, net::SocketAddr,
    sync::Arc,
};

use axum::{
    extract::ConnectInfo,
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{sse::Event, IntoResponse, Response, Sse},
    routing::{get, post, put},
    Json, Router,
};
use beam_lib::AppOrProxyId;
use futures_core::{stream, Stream};
use hyper::HeaderMap;
use serde::Deserialize;
use beam_lib::WorkStatus;
use shared::{
    config, errors::SamplyBeamError, sse_event::SseEventType,
    EncryptedMsgTaskRequest, EncryptedMsgTaskResult, HasWaitId, HowLongToBlock, Msg, MsgEmpty,
    MsgId, MsgSigned, MsgTaskRequest, MsgTaskResult, EMPTY_VEC_APPORPROXYID, serde_helpers::DerefSerializer,
};
use tokio::{
    sync::{
        broadcast::{Receiver, Sender},
        RwLock,
    },
    time,
};
use tracing::{debug, error, info, trace, warn};

use crate::task_manager::TaskManager;

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
) -> Result<DerefSerializer, StatusCode> {
    debug!(
        "get_results_for_task(task={}) called by {} with IP {addr}, wait={:?}",
        task_id.to_string(),
        msg.get_from(),
        block
    );
    if msg.get_from() != state.task_manager.get(&task_id)?.get_from() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let filter_for_me = MsgFilterNoTask {
        from: None,
        to: Some(msg.get_from().clone()),
        mode: MsgFilterMode::Or,
    };
    let task_with_results = state.task_manager.wait_for_results(&task_id, &block, |m| filter_for_me.matches(&m.msg)).await?;
    
    DerefSerializer::new(task_with_results.msg.results.values().filter(|m| filter_for_me.matches(&m.msg)), block.wait_count).map_err(|e| {
        warn!("Failed to serialize task results: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })
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
    if &from != state.task_manager.get(&task_id)?.get_from() {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let filter = MsgFilterNoTask { from: None, to: Some(from), mode: MsgFilterMode::Or };
    let stream = state.task_manager.stream_results(
        task_id,
        block,
        move |m| filter.matches(&m.msg)
    );

    Ok(Sse::new(stream))
}


#[derive(Deserialize)]
struct TaskFilter {
    from: Option<AppOrProxyId>,
    to: Option<AppOrProxyId>,
    filter: Option<FilterParam>,
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
    Query(taskfilter): Query<TaskFilter>,
    State(state): State<TasksState>,
    msg: MsgSigned<MsgEmpty>,
) -> Result<DerefSerializer, (StatusCode, impl IntoResponse)> {
    let from = taskfilter.from;
    let mut to = taskfilter.to;
    let unanswered_by = match taskfilter.filter {
        Some(FilterParam::Todo) => {
            if to.is_none() {
                to = Some(msg.get_from().clone());
            }
            Some(msg.get_from().clone())
        }
        None => None,
    };
    if from.is_none() && to.is_none() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Please supply either \"from\" or \"to\" query parameter.",
        ));
    }
    if (from.is_some() && *from.as_ref().unwrap() != msg.msg.from)
        || (to.is_some() && *to.as_ref().unwrap() != msg.msg.from)
    {
        // Rewrite in Rust 1.64: https://github.com/rust-lang/rust/pull/94927
        return Err((
            StatusCode::UNAUTHORIZED,
            "You can only list messages created by you (from) or directed to you (to).",
        ));
    }
    // Step 1: Get initial vector fill from HashMap + receiver for new elements
    let filter = MsgFilterNoTask {
        from,
        to,
        mode: MsgFilterMode::Or,
    };
    let filter = MsgFilterForTask {
        normal: filter,
        unanswered_by: unanswered_by.as_ref(),
        workstatus_is_not: [WorkStatus::Succeeded, WorkStatus::PermFailed]
            .iter()
            .map(std::mem::discriminant)
            .collect(),
    };
    let tasks = state.task_manager
        .wait_for_tasks(&block, move |m| filter.matches(m))
        .await?;
    DerefSerializer::new(tasks, block.wait_count).map_err(|e| {
        warn!("Failed to serialize tasks: {e}");
        (StatusCode::INTERNAL_SERVER_ERROR, "Failed to serialize tasks")
    })
}

trait MsgFilterTrait<M: Msg> {
    // fn new() -> Self;
    fn from(&self) -> Option<&AppOrProxyId>;
    fn to(&self) -> Option<&AppOrProxyId>;
    fn mode(&self) -> &MsgFilterMode;

    fn matches(&self, msg: &M) -> bool {
        match self.mode() {
            MsgFilterMode::Or => self.filter_or(msg),
            MsgFilterMode::And => self.filter_and(msg),
        }
    }

    /// Returns true iff the from or the to conditions match (or both)
    fn filter_or(&self, msg: &M) -> bool {
        if self.from().is_none() && self.to().is_none() {
            return true;
        }
        if let Some(to) = &self.to() {
            if msg.get_to().contains(to) {
                return true;
            }
        }
        if let Some(from) = &self.from() {
            if msg.get_from() == *from {
                return true;
            }
        }
        false
    }

    /// Returns true iff all defined from/to conditions are met.
    fn filter_and(&self, msg: &M) -> bool {
        if self.from().is_none() && self.to().is_none() {
            return true;
        }
        if let Some(to) = self.to() {
            if !msg.get_to().contains(to) {
                return false;
            }
        }
        if let Some(from) = &self.from() {
            if msg.get_from() != *from {
                return false;
            }
        }
        true
    }
}

#[allow(dead_code)]
enum MsgFilterMode {
    Or,
    And,
}
struct MsgFilterNoTask {
    from: Option<AppOrProxyId>,
    to: Option<AppOrProxyId>,
    mode: MsgFilterMode,
}

struct MsgFilterForTask<'a> {
    normal: MsgFilterNoTask,
    unanswered_by: Option<&'a AppOrProxyId>,
    workstatus_is_not: Vec<Discriminant<WorkStatus>>,
}

impl<'a> MsgFilterForTask<'a> {
    fn unanswered(&self, msg: &EncryptedMsgTaskRequest) -> bool {
        if self.unanswered_by.is_none() {
            debug!("Is {} unanswered? Yes, criterion not defined.", msg.id());
            return true;
        }
        let unanswered = self.unanswered_by.unwrap();
        for res in msg.results.values() {
            if res.get_from() == unanswered
                && self
                    .workstatus_is_not
                    .contains(&std::mem::discriminant(&res.msg.status))
            {
                debug!("Is {} unanswered? No, answer found.", msg.id());
                return false;
            }
        }
        debug!("Is {} unanswered? Yes, no matching answer found.", msg.id());
        true
    }
}

impl<'a> MsgFilterTrait<EncryptedMsgTaskRequest> for MsgFilterForTask<'a> {
    fn from(&self) -> Option<&AppOrProxyId> {
        self.normal.from.as_ref()
    }

    fn to(&self) -> Option<&AppOrProxyId> {
        self.normal.to.as_ref()
    }

    fn matches(&self, msg: &EncryptedMsgTaskRequest) -> bool {
        MsgFilterNoTask::matches(&self.normal, msg) && self.unanswered(&msg)
    }

    fn mode(&self) -> &MsgFilterMode {
        &self.normal.mode
    }
}

impl<'a, M: Msg> MsgFilterTrait<M> for MsgFilterNoTask {
    fn from(&self) -> Option<&AppOrProxyId> {
        self.from.as_ref()
    }

    fn to(&self) -> Option<&AppOrProxyId> {
        self.to.as_ref()
    }

    fn mode(&self) -> &MsgFilterMode {
        &self.mode
    }
}

// POST /v1/tasks
async fn post_task(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<TasksState>,
    msg: MsgSigned<EncryptedMsgTaskRequest>,
) -> Result<(StatusCode, impl IntoResponse), StatusCode> {
        // let id = MsgId::new();
    // msg.id = id;
    // TODO: Check if ID is taken
    trace!(
        "Client {} with IP {addr} is creating task {:?}",
        msg.msg.from, msg
    );
    let id = msg.msg.id;
    state.task_manager.post_task(msg)?;
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


    let status = if state.task_manager.put_result(&task_id, result)? {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CREATED
    };
    Ok(status)
}

#[cfg(all(test, never))] // Removed until the errors down below are fixed
mod test {
    use serde_json::Value;
    use shared::{
        beam_id::{AppId, AppOrProxyId, BeamId, BrokerId, ProxyId},
        EncryptedMsgTaskRequest, Msg, MsgSigned, MsgTaskRequest, MsgTaskResult, WorkStatus,
    };

    use super::{MsgFilterForTask, MsgFilterMode, MsgFilterNoTask, MsgFilterTrait};

    #[test]
    fn filter_task() {
        const BROKER_ID: &str = "broker";
        BrokerId::set_broker_id(BROKER_ID.into());
        let broker = BrokerId::new(BROKER_ID).unwrap();
        let proxy = ProxyId::random(&broker);
        let app1: AppOrProxyId = AppId::new(&format!("app1.{}", proxy)).unwrap().into();
        let app2: AppOrProxyId = AppId::new(&format!("app2.{}", proxy)).unwrap().into();
        let task = MsgTaskRequest::new(
            app1.clone(),
            vec![app2.clone()],
            "Important task".into(),
            shared::FailureStrategy::Retry {
                backoff_millisecs: 1000,
                max_tries: 5,
            },
            Value::Null,
        );
        let result_by_app2 = MsgTaskResult {
            from: app2.clone(),
            to: vec![task.get_from().clone()],
            task: *task.id(),
            status: WorkStatus::TempFailed,
            metadata: Value::Null,
            body: Some("I'd like to retry, please re-send this task".into()),
        };
        let result_by_app2 = MsgSigned {
            msg: result_by_app2,
            sig: "Certainly valid".into(),
        };
        let mut task = MsgSigned {
            msg: task,
            sig: "Certainly valid".into(),
        };
        // let a = app1.clone();
        let filter = MsgFilterNoTask {
            from: None,
            to: Some(&app2),
            mode: MsgFilterMode::Or,
        };
        let filter = MsgFilterForTask {
            normal: filter,
            unanswered_by: Some(&app2),
            workstatus_is_not: [WorkStatus::Succeeded, WorkStatus::PermFailed]
                .iter()
                .map(std::mem::discriminant)
                .collect(),
        };
        assert_eq!(
            filter.matches(&task),
            true,
            "There are no results yet, so I should get the task: {:?}",
            task
        );
        task.msg
            .results
            .insert(result_by_app2.get_from().clone(), result_by_app2);
        assert_eq!(
            filter.matches(&task),
            true,
            "The only result is TempFailed, so I should still get it: {:?}",
            task
        );

        let result_by_app2 = task.msg.results.get_mut(&app2).unwrap();
        result_by_app2.msg.status = WorkStatus::Succeeded;
        assert_eq!(
            filter.matches(&task),
            false,
            "It's done, so I shouldn't get it"
        );
    }
}
