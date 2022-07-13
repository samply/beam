use std::{collections::HashMap, sync::Arc};

use axum::{
    http::{StatusCode, header},
    routing::{get, post},
    Extension, Json, Router, extract::{Query, Path}, response::IntoResponse
};
use serde::{Deserialize};
use shared::{MsgTaskRequest, MsgTaskResult, MsgId, HowLongToBlock, HasWaitId, MsgSigned, MsgEmpty, Msg, EMPTY_VEC_APPORPROXYID, config, beam_id::AppOrProxyId};
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
        .route("/v1/tasks/:task_id/results", get(get_results_for_task).post(post_result))
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
    let filter_for_me = MsgFilter { from: None, to: Some(msg.get_from()), mode: MsgFilterMode::Or };
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
        wait_for_elements(&mut results, &block, rx, filter_for_me).await;
    }
    let statuscode = wait_get_statuscode(&results, &block);
    Ok((statuscode, Json(results)))
}

fn would_wait_for_elements<S>(vec: &Vec<S>, block: &HowLongToBlock) -> bool {
    usize::from(block.poll_count.unwrap_or(0)) > vec.len()
}

fn wait_get_statuscode<S>(vec: &Vec<S>, block: &HowLongToBlock) -> StatusCode {
    if usize::from(block.poll_count.unwrap_or(0)) > vec.len() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    }
}

async fn wait_for_elements<'a, K,M: Msg>(vec: &mut Vec<M>, block: &HowLongToBlock, mut new_element_rx: Receiver<M>, filter: MsgFilter<'a>)
where M: Clone + HasWaitId<K>, K: PartialEq
{
    let wait_until =
        time::Instant::now() + block.poll_timeout.unwrap_or(time::Duration::from_secs(31536000));
    trace!(
        "Now is {:?}. Will wait until {:?}",
        time::Instant::now(),
        wait_until
    );
    while usize::from(block.poll_count.unwrap_or(0)) > vec.len()
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
                        if filter.filter(&req) {
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
struct ToFromParam {
    from: Option<AppOrProxyId>,
    to: Option<AppOrProxyId>,
}

/// GET /v1/tasks
/// Will retrieve tasks that are at least FROM or TO the supplied parameters.
async fn get_tasks(
    block: HowLongToBlock,
    Query(to_from): Query<ToFromParam>,
    msg: MsgSigned<MsgEmpty>,
    Extension(state): Extension<State>,
) -> Result<(StatusCode, impl IntoResponse),(StatusCode, impl IntoResponse)> {
    let from = to_from.from;
    let to = to_from.to;
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
    let filter_from_or_for_me = MsgFilter { from: from.as_ref(), to: to.as_ref(), mode: MsgFilterMode::Or };
    let (mut vec, new_task_rx) = {
        let map = state.tasks.read().await;
        let vec: Vec<MsgSigned<MsgTaskRequest>> = map
            .iter()
            .filter_map(|(_,v)| if filter_from_or_for_me.filter(v) { Some(v.clone()) } else { None })
            .collect();
        (vec, state.new_task_tx.subscribe())
    };
    // Step 2: Extend vector with new elements, waiting for `block` amount of time/items
    wait_for_elements(&mut vec, &block, new_task_rx, filter_from_or_for_me).await;
    let statuscode = wait_get_statuscode(&vec, &block);
    Ok((statuscode, Json(vec)))
}

#[allow(dead_code)]
enum MsgFilterMode { Or, And }
struct MsgFilter<'a> {
    from: Option<&'a AppOrProxyId>,
    to: Option<&'a AppOrProxyId>,
    mode: MsgFilterMode
}

impl<'a> MsgFilter<'a> {
    fn filter<M: Msg>(&self, msg: &M) -> bool {
        match self.mode {
            MsgFilterMode::Or => self.filter_or(msg),
            MsgFilterMode::And => self.filter_and(msg)
        }
    }

    /// Returns true iff the from or the to conditions match (or both)
    fn filter_or<M: Msg>(&self, msg: &M) -> bool {
        if self.from.is_none() && self.to.is_none() {
            return true;
        }
        if let Some(to) = &self.to {
            if msg.get_to().contains(to) {
                return true;
            }
        }
        if let Some(from) = &self.from {
            if msg.get_from() == *from {
                return true;
            }
        }
        false
    }

    /// Returns true iff all defined from/to conditions are met.
    fn filter_and<M: Msg>(&self, msg: &M) -> bool {
        if self.from.is_none() && self.to.is_none() {
            return true;
        }
        if let Some(to) = self.to {
            if ! msg.get_to().contains(to) {
                return false;
            }
        }
        if let Some(from) = &self.from {
            if msg.get_from() != *from {
                return false;
            }
        }
        true
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

// POST /v1/tasks/:task_id/results
async fn post_result(
    Path(task_id): Path<MsgId>,
    result: MsgSigned<MsgTaskResult>,
    Extension(state): Extension<State>
) -> Result<(StatusCode, impl IntoResponse), (StatusCode, &'static str)> {
    debug!("Called: Task {:?}, {:?}", task_id, result);
    if task_id != result.msg.task {
        return Err((StatusCode::BAD_REQUEST, "Task IDs supplied in path and payload do not match."));
    }
    let worker_id = result.msg.from.clone();

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
    // result.msg.id = MsgId::new();
    // TODO: Check if ID exists
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
    Ok((
        statuscode,
        [(header::LOCATION, format!("/v1/tasks/{}/results/{}", task_id, worker_id))]
    ))
}