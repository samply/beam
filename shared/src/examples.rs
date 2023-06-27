use std::collections::HashMap;

use rand::Rng;
use serde_json::json;

use beam_lib::{AppId, ProxyId};
use crate::{
    config, FailureStrategy, MsgId, MsgSigned, MsgTaskRequest, MsgTaskResult,
};

type BrokerId = String;

#[cfg(debug_assertions)]
pub fn print_example_objects() -> bool {
    if std::env::args().nth(1).unwrap_or_default() == "examples" {
        let broker_id = std::env::args().nth(2);
        let proxy_id = match std::env::args().nth(3) {
            Some(id) => ProxyId::new(&id).ok(),
            None => None,
        };
        let (tasks, results) = generate_example_tasks(broker_id, proxy_id);
        for (num, task) in tasks.iter().enumerate() {
            println!(
                "export TASK{}='{}'",
                num,
                serde_json::to_string(task).unwrap().replace('\'', "\'")
            );
        }
        for (num, result) in results.iter().enumerate() {
            println!(
                "export RESULT{}='{}'",
                num,
                serde_json::to_string(&result).unwrap().replace('\'', "\'")
            );
        }
        true
    } else {
        false
    }
}

pub fn generate_example_tasks(
    broker: Option<BrokerId>,
    proxy: Option<ProxyId>,
) -> (Vec<MsgTaskRequest>, Vec<MsgTaskResult>) {
    let broker =
        broker.unwrap_or(config::CONFIG_SHARED.broker_domain.clone());
    let proxy = {
        if let Some(id) = proxy {
            if !id.can_be_signed_by(&broker) {
                panic!(
                    "Submitted proxy_id ({id}) cannot be signed by submitted broker_id ({broker})"
                );
            }
            id
        } else {
            ProxyId::new(&format!("proxy{}.{}", 23, broker)).unwrap()
        }
    };
    let app1 = AppId::new(&format!("app1.{proxy}")).unwrap();
    let app2 = AppId::new(&format!("app2.{proxy}")).unwrap();

    let task_for_apps_1_2 = MsgTaskRequest::new(
        app1.clone().into(),
        vec![app1.clone().into(), app2.clone().into()],
        "My important task".into(),
        FailureStrategy::Retry {
            backoff_millisecs: 1000,
            max_tries: 5,
        },
        json!(["The", "Broker", "can", "read", "and", "filter", "this"]),
    );

    let response_by_app1 = MsgTaskResult {
        from: app1.clone().into(),
        to: vec![app1.clone().into()],
        task: task_for_apps_1_2.id,
        status: beam_lib::WorkStatus::Succeeded,
        body: "All done!".into(),
        metadata: json!("A normal string works, too!"),
    };
    let response_by_app2 = MsgTaskResult {
        from: app2.into(),
        to: vec![app1.into()],
        task: task_for_apps_1_2.id,
        status: beam_lib::WorkStatus::PermFailed,
        body: "Unable to complete".into(),
        metadata: json!({ "I": { "like": [ "results", "cake" ] } }),
    };
    let mut tasks = Vec::new();
    for task in [task_for_apps_1_2] {
        tasks.push(task);
    }
    let mut results = Vec::new();
    for result in [response_by_app1, response_by_app2] {
        results.push(result);
    }
    (tasks, results)
}

// Random


fn random_str() -> String {
    const LENGTH: u8 = 8;
    const CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::thread_rng();
    let random_str: String = (0..=LENGTH)
        .map(|_| {
            let idx = rng.gen_range(0..CHARSET.len());
            CHARSET[idx] as char
        })
        .collect();
    random_str
}

fn random_number() -> u8 {
    let mut rng = rand::thread_rng();
    rng.gen_range(u8::MIN..u8::MAX)
}
