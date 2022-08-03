use std::{collections::HashMap, sync::Arc, mem::Discriminant};

use axum::{
    http::{StatusCode, header},
    routing::{get, post, put},
    Extension, Json, Router, extract::{Query, Path}, response::IntoResponse
};
use serde::{Deserialize};
use shared::{MsgTaskRequest, MsgTaskResult, MsgId, HowLongToBlock, HasWaitId, MsgSigned, MsgEmpty, Msg, EMPTY_VEC_APPORPROXYID, config, beam_id::AppOrProxyId, WorkStatus};
use tokio::{sync::{broadcast::{Sender, Receiver}, RwLock}, time};
use tracing::{debug, info, trace};

#[derive(Clone)]
struct State {
    tasks: Arc<RwLock<HashMap<MsgId, MsgSigned<MsgTaskRequest>>>>,
    new_task_tx: Arc<Sender<MsgSigned<MsgTaskRequest>>>,
    new_result_tx: Arc<RwLock<HashMap<MsgId, Sender<MsgSigned<MsgTaskResult>>>>>,
}

pub(crate) fn router() -> Router {
    Router::new()
        .route("/v1/tasks", get(get_tasks).post(post_task))
        .route("/v1/tasks/:task_id/results", get(get_results_for_task))
        .route("/v1/tasks/:task_id/results/:app_id", put(put_result))
        .layer(Extension(State::default()))
}

impl Default for State {
    fn default() -> Self {
        let tasks: HashMap<MsgId, MsgSigned<MsgTaskRequest>> = HashMap::new();
        let (new_task_tx, _) = tokio::sync::broadcast::channel::<MsgSigned<MsgTaskRequest>>(512);
    
        let tasks = Arc::new(RwLock::new(tasks));
        let new_task_tx = Arc::new(new_task_tx);
        State { tasks, new_task_tx, new_result_tx: Arc::new(RwLock::new(HashMap::new())) }
    }
}

// GET /v1/tasks/:task_id/results
async fn get_results_for_task(
    block: HowLongToBlock,
    task_id: MsgId,
    msg: MsgSigned<MsgEmpty>,
    Extension(state): Extension<State>,
) -> Result<(StatusCode, Json<Vec<MsgSigned<MsgTaskResult>>>), (StatusCode, &'static str)> {
    debug!("get_results_for_task called by {}: {:?}, {:?}", msg.get_from(), task_id, block);
    let filter_for_me = MsgFilterNoTask { from: None, to: Some(msg.get_from()), mode: MsgFilterMode::Or };
    let (mut results, rx)  = {
        let tasks = state.tasks.read().await;
        let task = match tasks.get(&task_id) {
            Some(task) => task,
            None => return Err((StatusCode::NOT_FOUND, "Task not found")),
        };
        if task.get_from() != msg.get_from() {
            return Err((StatusCode::UNAUTHORIZED, "Not your task."));
        }
        let results = task.msg.results.values().cloned().collect();
        let rx = match would_wait_for_elements(&results, &block) {
            true => Some(state.new_result_tx.read().await.get(&task_id)
                        .unwrap_or_else(|| panic!("Internal error: No result_tx found for task {}", task_id))
                        .subscribe()),
            false => None,
        };
        (results, rx)
    };
    if let Some(rx) = rx {
        wait_for_elements_notask(&mut results, &block, rx, &filter_for_me).await;
    }
    let statuscode = wait_get_statuscode(&results, &block);
    Ok((statuscode, Json(results)))
}

fn would_wait_for_elements<S>(vec: &Vec<S>, block: &HowLongToBlock) -> bool {
    usize::from(block.wait_count.unwrap_or(0)) > vec.len()
}

fn wait_get_statuscode<S>(vec: &Vec<S>, block: &HowLongToBlock) -> StatusCode {
    if usize::from(block.wait_count.unwrap_or(0)) > vec.len() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    }
}

// TODO: Is there a way to write this function in a generic way? (1/2)
async fn wait_for_elements_notask<'a, K,M: Msg>(vec: &mut Vec<M>, block: &HowLongToBlock, mut new_element_rx: Receiver<M>, filter: &MsgFilterNoTask<'a>)
where M: Clone + HasWaitId<K>, K: PartialEq
{
    let wait_until =
        time::Instant::now() + block.wait_time.unwrap_or(time::Duration::from_secs(31536000));
    trace!(
        "Now is {:?}. Will wait until {:?}",
        time::Instant::now(),
        wait_until
    );
    while usize::from(block.wait_count.unwrap_or(0)) > vec.len()
        && time::Instant::now() < wait_until {
        trace!(
            "Items in vec: {}, time remaining: {:?}",
            vec.len(),
            wait_until - time::Instant::now()
        );
        tokio::select! {
            _ = tokio::time::sleep_until(wait_until) => {
                break;
            },
            result = new_element_rx.recv() => {
                match result {
                    Ok(req) => {
                        if filter.matches(&req) {
                            vec.retain(|el| el.get_wait_id() != req.get_wait_id());
                            vec.push(req);
                        }
                    },
                    Err(_) => { panic!("Unable to receive from queue! What happened?"); }
                }
            }
        }
    }
}

// TODO: Is there a way to write this function in a generic way? (2/2)
async fn wait_for_elements_task<'a>(vec: &mut Vec<MsgSigned<MsgTaskRequest>>, block: &HowLongToBlock, mut new_element_rx: Receiver<MsgSigned<MsgTaskRequest>>, filter: &MsgFilterForTask<'a>)
{
    let wait_until =
        time::Instant::now() + block.wait_time.unwrap_or(time::Duration::from_secs(31536000));
    trace!(
        "Now is {:?}. Will wait until {:?}",
        time::Instant::now(),
        wait_until
    );
    while usize::from(block.wait_count.unwrap_or(0)) > vec.len()
        && time::Instant::now() < wait_until {
        trace!(
            "Items in vec: {}, time remaining: {:?}",
            vec.len(),
            wait_until - time::Instant::now()
        );
        tokio::select! {
            _ = tokio::time::sleep_until(wait_until) => {
                break;
            },
            result = new_element_rx.recv() => {
                match result {
                    Ok(req) => {
                        if filter.matches(&req) {
                            vec.retain(|el| el.get_wait_id() != req.get_wait_id());
                            vec.push(req);
                        }
                    },
                    Err(_) => { panic!("Unable to receive from queue! What happened?"); }
                }
            }
        }
    }
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
    Todo
}

/// GET /v1/tasks
/// Will retrieve tasks that are at least FROM or TO the supplied parameters.
async fn get_tasks(
    block: HowLongToBlock,
    Query(taskfilter): Query<TaskFilter>,
    msg: MsgSigned<MsgEmpty>,
    Extension(state): Extension<State>,
) -> Result<(StatusCode, impl IntoResponse),(StatusCode, impl IntoResponse)> {
    let from = taskfilter.from;
    let mut to = taskfilter.to;
    let unanswered_by = match taskfilter.filter {
        Some(FilterParam::Todo) => {
            if to.is_none() {
                to = Some(msg.get_from().clone());
            }
            Some(msg.get_from().clone())
        },
        None => None,
    };
    if from.is_none() && to.is_none() {
        return Err((StatusCode::BAD_REQUEST, "Please supply either \"from\" or \"to\" query parameter."));
    }
    debug!(?from);
    debug!(?to);
    debug!(?msg);
    if (from.is_some() && *from.as_ref().unwrap() != msg.msg.from) 
    || (to.is_some() && *to.as_ref().unwrap() != msg.msg.from) { // Rewrite in Rust 1.64: https://github.com/rust-lang/rust/pull/94927
        return Err((StatusCode::UNAUTHORIZED, "You can only list messages created by you (from) or directed to you (to)."));
    }
    // Step 1: Get initial vector fill from HashMap + receiver for new elements
    let filter = MsgFilterNoTask { from: from.as_ref(), to: to.as_ref(), mode: MsgFilterMode::Or };
    let filter = MsgFilterForTask { 
        normal: filter,
        unanswered_by: unanswered_by.as_ref(),
        workstatus_is_not: 
            [WorkStatus::Succeeded(String::new()), WorkStatus::PermFailed(String::new())]
            .iter().map(std::mem::discriminant).collect()
    };
    let (mut vec, new_task_rx) = {
        let map = state.tasks.read().await;
        let vec: Vec<MsgSigned<MsgTaskRequest>> = map
            .iter()
            .filter_map(|(_,v)|
                if filter.matches(v) {
                    Some(v.clone()) 
                } else { 
                    None 
                })
            .collect();
        (vec, state.new_task_tx.subscribe())
    };
    // Step 2: Extend vector with new elements, waiting for `block` amount of time/items
    wait_for_elements_task(&mut vec, &block, new_task_rx, &filter).await;
    let statuscode = wait_get_statuscode(&vec, &block);
    Ok((statuscode, Json(vec)))
}

trait MsgFilterTrait<M: Msg> {
    // fn new() -> Self;
    fn from(&self) -> Option<&AppOrProxyId>;
    fn to(&self) -> Option<&AppOrProxyId>;
    fn mode(&self) -> &MsgFilterMode;

    fn matches(&self, msg: &M) -> bool {
        match self.mode() {
            MsgFilterMode::Or => self.filter_or(msg),
            MsgFilterMode::And => self.filter_and(msg)
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
            if ! msg.get_to().contains(to) {
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
enum MsgFilterMode { Or, And }
struct MsgFilterNoTask<'a> {
    from: Option<&'a AppOrProxyId>,
    to: Option<&'a AppOrProxyId>,
    mode: MsgFilterMode
}

struct MsgFilterForTask<'a> {
    normal: MsgFilterNoTask<'a>,
    unanswered_by: Option<&'a AppOrProxyId>,
    workstatus_is_not: Vec<Discriminant<WorkStatus>>
}

impl<'a> MsgFilterForTask<'a> {
    fn unanswered(&self, msg: &MsgTaskRequest) -> bool {
        if self.unanswered_by.is_none() {
            debug!("Is {} unanswered? Yes, criterion not defined.", msg.id);
            return true;
        }
        let unanswered = self.unanswered_by.unwrap();
        for res in msg.results.values() {
            if res.get_from() == unanswered && self.workstatus_is_not.contains(&std::mem::discriminant(&res.msg.status)) {
                debug!("Is {} unanswered? No, answer found.", msg.id);
                return false;
            }
        }
        debug!("Is {} unanswered? Yes, no matching answer found.", msg.id);
        true
    }
}

impl<'a> MsgFilterTrait<MsgSigned<MsgTaskRequest>> for MsgFilterForTask<'a> {
    fn from(&self) -> Option<&AppOrProxyId> {
        self.normal.from
    }

    fn to(&self) -> Option<&AppOrProxyId> {
        self.normal.to
    }

    fn matches(&self, msg: &MsgSigned<MsgTaskRequest>) -> bool {
        MsgFilterNoTask::matches(&self.normal, msg)
            && self.unanswered(&msg.msg)
    }

    fn mode(&self) -> &MsgFilterMode {
        &self.normal.mode
    }
}

impl<'a, M: Msg> MsgFilterTrait<M> for MsgFilterNoTask<'a> {
    fn from(&self) -> Option<&AppOrProxyId> {
        self.from
    }

    fn to(&self) -> Option<&AppOrProxyId> {
        self.to
    }

    fn mode(&self) -> &MsgFilterMode {
        &self.mode
    }
}

// POST /v1/tasks
async fn post_task(
    msg: MsgSigned<MsgTaskRequest>,
    Extension(state): Extension<State>,
) -> Result<(StatusCode, impl IntoResponse), (StatusCode, String)> {
    // let id = MsgId::new();
    // msg.id = id;
    // TODO: Check if ID is taken
    debug!("Client {} is creating task {:?}", msg.msg.from, msg);
    let (new_tx, _) = tokio::sync::broadcast::channel(256);
    {
        let mut tasks = state.tasks.write().await;
        let mut txes = state.new_result_tx.write().await;
        if tasks.contains_key(&msg.msg.id) {
            return Err((StatusCode::CONFLICT, format!("ID {} is already taken.", msg.msg.id)));
        }
        tasks.insert(msg.msg.id, msg.clone());
        txes.insert(msg.msg.id, new_tx);
        if let Err(e) = state.new_task_tx.send(msg.clone()) {
            debug!("Unable to send notification: {}. Ignoring since probably noone is currently waiting for tasks.", e);
        }
    }
    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, format!("/v1/tasks/{}", msg.msg.id))]
    ))
}

// PUT /v1/tasks/:task_id/results/:app_id
async fn put_result(
    Path((task_id, app_id)): Path<(MsgId,AppOrProxyId)>,
    result: MsgSigned<MsgTaskResult>,
    Extension(state): Extension<State>
) -> Result<StatusCode, (StatusCode, &'static str)> {
    debug!("Called: Task {:?}, {:?}", task_id, result);
    if task_id != result.msg.task {
        return Err((StatusCode::BAD_REQUEST, "Task IDs supplied in path and payload do not match."));
    }
    let worker_id = result.msg.from.clone();
    if app_id != worker_id {
        return Err((StatusCode::BAD_REQUEST, "AppID supplied in URL and signed message do not match."));
    }

    // Step 1: Check prereqs.
    let mut tasks = state.tasks.write().await;

    // TODO: Check if this can be written nicer using .entry()
    let task = match tasks.get_mut(&task_id) {
        Some(task) => &mut task.msg,
        None => { return Err((StatusCode::NOT_FOUND, "Task not found")) },
    };
    debug!(?task, ?worker_id, "Checking if task is in worker ID: ");
    if ! task.to.contains(&worker_id) {
        return Err((StatusCode::UNAUTHORIZED, "Your result is not requested for this task."));
    }

    // Step 2: Insert.
    let statuscode = match task.results.insert(worker_id.clone(), result.clone()) {
        Some(_) => StatusCode::NO_CONTENT,
        None => StatusCode::CREATED,
    };

    // Step 3: Notify. This has to happen while the lock for tasks is still held since otherwise results could get lost.
    let sender =
        state.new_result_tx.read().await;
    let sender = sender
        .get(&task_id)
        .unwrap_or_else(|| panic!("Internal error: No result_tx found for task {}", task_id));
    if let Err(e) = sender.send(result) {
        debug!("Unable to send notification: {}. Ignoring since probably noone is currently waiting for tasks.", e);
    }
    Ok(statuscode)
}

#[cfg(test)]
mod test {
    use serde_json::Value;
    use shared::{MsgTaskRequest, beam_id::{AppId, ProxyId, BrokerId, BeamId, AppOrProxyId}, MsgTaskResult, Msg, WorkStatus, MsgSigned};

    use super::{MsgFilterForTask, MsgFilterNoTask, MsgFilterMode, MsgFilterTrait};

    #[test]
    fn filter_task() {
        const broker_id: &str = "broker";
        BrokerId::set_broker_id(broker_id.into());
        let broker = BrokerId::new(broker_id).unwrap();
        let proxy = ProxyId::random(&broker);
        let app1: AppOrProxyId = AppId::new(&format!("app1.{}", proxy)).unwrap().into();
        let app2: AppOrProxyId = AppId::new(&format!("app2.{}", proxy)).unwrap().into();
        let task = MsgTaskRequest::new(
            app1.clone(),
            vec![app2.clone()],
            "Important task".into(),
            shared::FailureStrategy::Retry { backoff_millisecs: 1000, max_tries: 5 },
            Value::Null
        );
        let result_by_app2 = MsgTaskResult::new(
            app2.clone(),
            vec![task.get_from().clone()],
            *task.get_id(),
            WorkStatus::TempFailed("I'd like to retry, please re-send this task".into()),
            Value::Null
        );
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
            workstatus_is_not: [WorkStatus::Succeeded(String::new()), WorkStatus::PermFailed(String::new())]
            .iter().map(std::mem::discriminant).collect(),
        };
        assert_eq!(filter.matches(&task), true, "There are no results yet, so I should get the task: {:?}", task);
        task.msg.results.insert(result_by_app2.get_from().clone(), result_by_app2);
        assert_eq!(filter.matches(&task), true, "The only result is TempFailed, so I should still get it: {:?}", task);

        let result_by_app2 = task.msg.results.get_mut(&app2).unwrap();
        *result_by_app2.msg.status_mut() = WorkStatus::Succeeded("I'm done!".into());
        assert_eq!(filter.matches(&task), false, "It's done, so I shouldn't get it");
    }
}
