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

mod serve_axum;
#[cfg(debug_assertions)]
mod devhelper;

use std::{collections::HashMap, sync::Arc};

use shared::*;
use tokio::sync::RwLock;

#[tokio::main]
pub async fn main() {
    #[cfg(debug_assertions)]
    if devhelper::print_example_objects() == true { return; }

    let tasks: HashMap<MsgId, MsgTaskRequest> = HashMap::new();
    let (new_tasks_tx, _) = tokio::sync::broadcast::channel::<MsgTaskRequest>(512);

    let tasks = Arc::new(RwLock::new(tasks));
    let new_tasks_tx = Arc::new(new_tasks_tx);

    serve_axum::serve_axum(tasks, new_tasks_tx).await;
}
