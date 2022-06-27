//! # Broker (Central Component)
//! ## API
//! POST /tasks/ ==> Create Task. Give MsgTask without ID, return task_uuid
//! GET /tasks/ ==> Get all tasks where I am "to"
//! PUT /tasks/{task_uuid}/results/{worker_uuid} ==> Update existing result
//! GET /tasks/{task_uuid}/results/ ==> Give all results for task uuid
//! GET /tasks/{task_uuid}/results/{worker_uuid} ==> Give that specific result
//!
//! ### Parameters:
//! ####  Long Polling
//! - (default) Don't block; return results immediately
//! - &block.timeout=1500ms => Return whatever is there after 1500ms
//! - &block.results=5 => Block forever until 5 results have arrived
//! - &block.timeout=1500ms&block.results=5 => Block for 1500ms OR if >= 5 results have arrived (whichever comes first)
//!
//! ### Headers:
//!  - Authorization: Basic(MyUUID, secretString)
//!

mod errors;

#[cfg(feature = "backend-warp")]
mod logic_warp;
#[cfg(feature = "backend-warp")]
mod serve_warp;

#[cfg(feature = "backend-axum")]
mod serve_axum;

use std::{collections::HashMap, sync::Arc};

use shared::*;
use tokio::sync::RwLock;

#[tokio::main]
pub async fn main() {
    #[cfg(debug_assertions)]
    if print_example_objects() == true { return; }

    let tasks: HashMap<MsgId, MsgTaskRequest> = HashMap::new();
    let (new_tasks_tx, _) = tokio::sync::broadcast::channel::<MsgTaskRequest>(512);

    let tasks = Arc::new(RwLock::new(tasks));
    let new_tasks_tx = Arc::new(new_tasks_tx);

    #[cfg(feature = "backend-warp")]
    serve_warp::serve_warp(tasks, new_tasks_tx).await;

    #[cfg(feature = "backend-axum")]
    serve_axum::serve_axum(tasks, new_tasks_tx).await;
}

#[cfg(debug_assertions)]
fn print_example_objects() -> bool {
    if std::env::args().nth(1).unwrap_or_default() == "examples" {
        let tasks = generate_example_tasks();
        let mut num_tasks = 0;
        let mut num_results = 0;
        for task in tasks.values() {
            println!("export TASK{}='{}'", num_tasks, serde_json::to_string(task).unwrap().replace("'", "\'"));
            num_tasks = num_tasks + 1;
            for result in task.results.values() {
                println!("export RESULT{}='{}'", num_results, serde_json::to_string(result).unwrap().replace("'", "\'"));
                num_results = num_results + 1;
            }
        }
        return true;
    } else {
        return false;
    }
}