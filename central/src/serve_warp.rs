use std::{collections::HashMap, convert::Infallible, sync::Arc};

use shared::*;
use tokio::sync::{
    broadcast::{Receiver, Sender},
    RwLock,
};
use warp::{hyper::StatusCode, reply, Filter, Rejection, Reply};

use crate::{errors::*, logic::*};

pub(crate) async fn serve_warp(
    requests: Arc<RwLock<HashMap<uuid::Uuid, MsgTaskRequest>>>,
    results: Arc<RwLock<HashMap<uuid::Uuid, MsgTaskResult>>>,
    new_tasks_tx: Arc<tokio::sync::broadcast::Sender<MsgTaskRequest>>,
) {
    let log = warp::log::custom(|info| {
        // Use a log macro, or slog, or println, or whatever!
        println!("{} {} {}", info.method(), info.path(), info.status(),);
    });

    let get_tasks = warp::get()
        .and(with_state(requests.clone()))
        .and(with_receiver(new_tasks_tx.clone()))
        .and(warp::path("tasks"))
        .and(warp::query())
        .and(warp::path::end())
        .and_then(get_tasks);

    let post_task = warp::post()
        .and(with_state(requests.clone()))
        .and(warp::path("tasks"))
        .and(warp::body::json())
        .and(with_state(new_tasks_tx))
        .and(warp::path::end())
        .and_then(new_task);

    let post_result = warp::get()
        .and(with_state(results))
        .and(warp::path("tasks"))
        .and(warp::path::param::<String>())
        .and(warp::path::param::<String>())
        .and(warp::body::json())
        .and(warp::path::end())
        .and_then(new_result);

    let routes = post_task
        .or(get_tasks)
        .or(post_result)
        .recover(handle_rejection)
        .with(log);

    warp::serve(routes).run(([127, 0, 0, 1], 8080)).await
}

fn with_state<S>(
    state: Arc<S>,
) -> impl Filter<Extract = (Arc<S>,), Error = std::convert::Infallible> + Clone
where
    S: Send + Sync,
{
    warp::any().map(move || state.clone())
}

fn with_receiver<S>(
    tx: Arc<Sender<S>>,
) -> impl Filter<Extract = (Receiver<S>,), Error = Infallible> + Clone
where
    S: 'static + Send + Sync,
{
    let tx = tx.clone();
    warp::any().map(move || tx.subscribe())
}

async fn handle_rejection(err: Rejection) -> Result<impl Reply, std::convert::Infallible> {
    println!("Handling rejection: {:?}", err);
    if err.is_not_found() {
        Ok(reply::with_status(
            "NOT_FOUND".to_string(),
            StatusCode::NOT_FOUND,
        ))
    } else if let Some(e) = err.find::<WebError>() {
        Ok(reply::with_status(e.reason.as_str().to_string(), e.code))
    } else {
        eprintln!("unhandled rejection: {:?}", err);
        Ok(reply::with_status(
            "INTERNAL_SERVER_ERROR".to_string(),
            StatusCode::INTERNAL_SERVER_ERROR,
        ))
    }
}
