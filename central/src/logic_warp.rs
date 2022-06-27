use std::{collections::HashMap, sync::Arc, time::Duration};

use shared::*;
use tokio::{
    sync::{
        broadcast::{Receiver, Sender},
        RwLock,
    },
    time,
};
use uuid::Uuid;
use warp::hyper::{header, StatusCode};

use crate::errors::WebError;

pub(crate) async fn new_task(
    requests: Arc<RwLock<HashMap<MsgId, MsgTaskRequest>>>,
    mut task: MsgTaskRequest,
    new_tasks_tx: Arc<Sender<MsgTaskRequest>>,
) -> Result<impl warp::Reply, warp::Rejection> {
    let id = Uuid::new_v4();
    task.id = id;
    {
        requests.write().await.insert(id, task.clone());
        if let Err(e) = new_tasks_tx.send(task) {
            println!("Unable to send notification: {}. Ignoring since probably noone is currently waiting for tasks.", e);
        }
    }
    let reply = warp::reply::json(&id);
    let reply = warp::reply::with_status(reply, StatusCode::CREATED);
    let reply = warp::reply::with_header(reply, header::LOCATION, format!("/tasks/{}", id));
    Ok(reply)
}

pub(crate) async fn get_tasks(
    requests: Arc<RwLock<HashMap<MsgId, MsgTaskRequest>>>,
    mut new_tasks_rx: Receiver<MsgTaskRequest>,
    params: HashMap<String, String>,
) -> Result<impl warp::Reply, warp::Rejection> {
    // Parse input
    let block = HowLongToBlock {
        resultcount: try_read(&params, "resultcount"),
        timeout: match try_read(&params, &"timeout") {
            Some(millisecs) => Some(Duration::from_millis(millisecs)),
            None => None,
        },
    };
    let party = try_read::<Uuid>(&params, "party");
    if party.is_none() {
        return Err(warp::reject::custom(WebError {
            reason: "Please supply your party ID.".to_string(),
            code: StatusCode::BAD_REQUEST,
        }));
    }
    let party = party.unwrap();

    // Begin collecting
    let wait_until =
        time::Instant::now() + block.timeout.unwrap_or(time::Duration::from_secs(31536000));
    println!(
        "Now is {:?}. Will wait until {:?}",
        time::Instant::now(),
        wait_until
    );

    let (vec, reply_status) = {
        let mut vec = {
            let map = requests.read().await;
            let vec: Vec<MsgTaskRequest> = map
                .iter()
                .filter_map(|(_, v)| match v.to.contains(&party) {
                    true => Some(v.clone()),
                    false => None,
                })
                .collect();
            new_tasks_rx = new_tasks_rx.resubscribe();
            vec
        };
        while usize::from(block.resultcount.unwrap_or(0)) > vec.len()
            && time::Instant::now() < wait_until
        {
            println!(
                "Items in vec: {}, time remaining: {:?}",
                vec.len(),
                wait_until - time::Instant::now()
            );
            tokio::select! {
                _ = tokio::time::sleep_until(wait_until) => {
                    break;
                },
                result = new_tasks_rx.recv() => {
                    match result {
                        Ok(req) => { vec.push(req); },
                        Err(_) => { panic!("Unable to receive from queue! What happened?"); }
                    }
                }
            }
        }
        if usize::from(block.resultcount.unwrap_or(0)) > vec.len() {
            (vec, StatusCode::PARTIAL_CONTENT)
        } else {
            (vec, StatusCode::OK)
        }
    };

    let reply = warp::reply::json(&vec);
    let reply = warp::reply::with_status(reply, reply_status);
    Ok(reply)
}

pub(crate) async fn new_result(
    results: Arc<RwLock<HashMap<MsgId, MsgTaskResult>>>,
    task_uuid: String,
    party_uuid: String,
    result: MsgTaskResult,
) -> Result<impl warp::Reply, warp::Rejection> {
    let task_uuid = Uuid::parse_str(&task_uuid);
    let party_uuid = Uuid::parse_str(&party_uuid);
    if task_uuid.is_err() || party_uuid.is_err() {
        return Err(warp::reject::custom(WebError {
            reason: "Please supply valid task and party UUIDs.".to_string(),
            code: StatusCode::BAD_REQUEST,
        }));
    }
    let (task_uuid, party_uuid) = (task_uuid.unwrap(), party_uuid.unwrap());

    // task.id = id;
    // {
    //     results.write().await.insert(id, task.clone());
    //     if let Err(e) = new_tasks_tx.send(task) {
    //         println!("Unable to send notification: {}. Ignoring since probably noone is currently waiting for tasks.", e);
    //     }
    // }
    // let reply = warp::reply::json(&id);
    // let reply = warp::reply::with_status(reply, StatusCode::CREATED);
    // let reply = warp::reply::with_header(reply, header::LOCATION, format!("/tasks/{}", id));
    // Ok(reply)
    let reply = warp::reply::json(&result);
    Ok(reply)
}
