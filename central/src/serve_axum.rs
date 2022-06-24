use std::{collections::HashMap, sync::Arc, hash::Hash};

use axum::{
    http::{StatusCode, header},
    routing::{get, put},
    Extension, Json, Router, extract::{Query, Path}, response::IntoResponse,
};
use serde::Deserialize;
use shared::{MsgTaskRequest, MsgTaskResult, MsgId, HowLongToBlock, ClientId, HasWaitId};
// use shared::{*};
use tokio::{sync::{broadcast::{Sender, Receiver}, RwLock}, time};

#[derive(Clone)]
struct State {
    tasks: Arc<RwLock<HashMap<MsgId, MsgTaskRequest>>>,
    new_task_tx: Arc<Sender<MsgTaskRequest>>,
    new_result_tx: Arc<RwLock<HashMap<MsgId, Sender<MsgTaskResult>>>>,
}

pub(crate) async fn serve_axum(
    tasks: Arc<RwLock<HashMap<MsgId, MsgTaskRequest>>>,
    new_task_tx: Arc<Sender<MsgTaskRequest>>,
) {
    let state = State { tasks, new_task_tx, new_result_tx: Arc::new(RwLock::new(HashMap::new())) };
    let app = Router::new()
        .route("/tasks", get(get_tasks).post(post_task))
        .route("/tasks/:task_id/results", get(get_results_for_task).post(post_result))
        .layer(Extension(state));
    axum::Server::bind(&"0.0.0.0:8080".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}

// GET /tasks/:task_id/results
async fn get_results_for_task(
    block: HowLongToBlock,
    task_id: MsgId,
    Extension(state): Extension<State>,
) -> Result<(StatusCode, Json<Vec<MsgTaskResult>>), (StatusCode, &'static str)> {
    println!("get_results_for_task called: {:?}, {:?}", task_id, block);
    let (mut results, rx)  = {
        let tasks = state.tasks.read().await;
        let task = match tasks.get(&task_id) {
            Some(task) => task,
            None => return Err((StatusCode::NOT_FOUND, "Task not found")),
        };
        let results = task.results.values().map(|v| v.clone()).collect();
        let rx = match would_wait_for_elements(&results, &block) {
            true => Some(state.new_result_tx.read().await.get(&task_id)
                        .expect(&format!("Internal error: No result_tx found for task {}", task_id))
                        .subscribe()),
            false => None,
        };
        (results, rx)
    };
    if let Some(rx) = rx {
        wait_for_elements(&mut results, &block, rx).await;
    }
    let statuscode = wait_get_statuscode(&results, &block);
    Ok((statuscode, Json(results)))
}

fn would_wait_for_elements<S>(vec: &Vec<S>, block: &HowLongToBlock) -> bool {
    usize::from(block.resultcount.unwrap_or(0)) > vec.len()
}

fn wait_get_statuscode<S>(vec: &Vec<S>, block: &HowLongToBlock) -> StatusCode {
    /*if vec.len() == 0 {
        StatusCode::NOT_FOUND
    } else*/ if usize::from(block.resultcount.unwrap_or(0)) > vec.len() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    }
}

async fn wait_for_elements<K,V>(vec: &mut Vec<V>, block: &HowLongToBlock, mut new_element_rx: Receiver<V>)
where V: Clone + HasWaitId<K>, K: PartialEq
{
    let wait_until =
        time::Instant::now() + block.timeout.unwrap_or(time::Duration::from_secs(31536000));
    println!(
        "Now is {:?}. Will wait until {:?}",
        time::Instant::now(),
        wait_until
    );
    while usize::from(block.resultcount.unwrap_or(0)) > vec.len()
        && time::Instant::now() < wait_until {
        println!(
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
                        vec.retain(|el| el.get_wait_id() != req.get_wait_id());
                        vec.push(req);
                    },
                    Err(_) => { panic!("Unable to receive from queue! What happened?"); }
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct WorkerIDParam {
    worker_id: Option<ClientId>,
}

// GET /tasks
async fn get_tasks(
    block: HowLongToBlock,
    Query(worker_id): Query<WorkerIDParam>,
    Extension(state): Extension<State>,
) -> (StatusCode, impl IntoResponse) {
    // Step 1: Get initial vector fill from HashMap + receiver for new elements
    let (mut vec, new_task_rx) = {
        let map = state.tasks.read().await;
        let worker_id = worker_id.worker_id;
        let vec: Vec<MsgTaskRequest> = map
            .iter()
            .filter_map(|(_, v)| {
                if worker_id.is_none() {
                    return Some(v.clone())
                };
                let worker_id = worker_id.unwrap();
                // let worker_id = worker_id.unwrap().worker_id.clone();
                match v.to.contains(&worker_id) {
                    true => Some(v.clone()),
                    false => None,
                }
            })
            .collect();
        (vec, state.new_task_tx.subscribe())
    };
    // Step 2: Extend vector with new elements, waiting for `block` amount of time/items
    wait_for_elements(&mut vec, &block, new_task_rx).await;
    let statuscode = wait_get_statuscode(&vec, &block);
    (statuscode, Json(vec))
}

// POST /tasks
async fn post_task(
    Json(mut task): Json<MsgTaskRequest>,
    Extension(state): Extension<State>,
) -> (StatusCode, impl IntoResponse) {
    let id = MsgId::new();
    task.id = id;
    let (new_tx, _) = tokio::sync::broadcast::channel(256);
    {
        let mut tasks = state.tasks.write().await;
        let mut txes = state.new_result_tx.write().await;
        tasks.insert(id, task.clone());
        txes.insert(id, new_tx);
        if let Err(e) = state.new_task_tx.send(task) {
            println!("Unable to send notification: {}. Ignoring since probably noone is currently waiting for tasks.", e);
        }
    }
    (
        StatusCode::CREATED,
        [(header::LOCATION, format!("/tasks/{}", id))]
    )
}

// POST /tasks/:task_id/results
async fn post_result(
    Path(task_id): Path<MsgId>,
    Json(mut result): Json<MsgTaskResult>,
    Extension(state): Extension<State>
) -> Result<(StatusCode, impl IntoResponse), (StatusCode, &'static str)> {
    println!("Called: Task {:?}, {:?}", task_id, result);
    if task_id != result.task {
        return Err((StatusCode::BAD_REQUEST, "Task IDs supplied in path and payload do not match."));
    }
    let worker_id = result.worker_id;

    // Step 1: Check prereqs.
    let mut tasks = state.tasks.write().await;

    // TODO: Check if this can be written nicer using .entry()
    let task = match tasks.get_mut(&task_id) {
        Some(task) => task,
        None => { return Err((StatusCode::NOT_FOUND, "Task not found")) },
    };
    if ! task.to.contains(&worker_id) {
        return Err((StatusCode::UNAUTHORIZED, "Your result is not requested for this task."));
    }

    // Step 2: Insert.
    result.id = MsgId::new();
    let statuscode = match task.results.insert(worker_id, result.clone()) {
        Some(_) => StatusCode::NO_CONTENT,
        None => StatusCode::CREATED,
    };

    // Step 3: Notify. This has to happen while the lock for tasks is still held since otherwise results could get lost.
    let sender =
        state.new_result_tx.read().await;
    let sender = sender
        .get(&task_id)
        .expect(&format!("Internal error: No result_tx found for task {}", task_id));
    if let Err(e) = sender.send(result) {
        println!("Unable to send notification: {}. Ignoring since probably noone is currently waiting for tasks.", e);
    }
    Ok((
        statuscode,
        [(header::LOCATION, format!("/tasks/{}/results/{}", task_id, worker_id))]
    ))
}
