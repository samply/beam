use std::collections::HashMap;

use rand::Rng;

use crate::{beam_id2::{BeamId, AppId, BrokerId, ProxyId}, MsgId, MsgTaskRequest, config, MsgTaskResult, MsgSigned, FailureStrategy};

#[cfg(debug_assertions)]
pub fn print_example_objects() -> bool {
    if std::env::args().nth(1).unwrap_or_default() == "examples" {
        let broker_id = match std::env::args().nth(2) {
            Some(id) => BrokerId::new(&id).ok(),
            None => None,
        };
        let proxy_id = match std::env::args().nth(3) {
            Some(id) => ProxyId::new(&id).ok(),
            None => None,
        };
        let tasks = generate_example_tasks(broker_id, proxy_id);
        let mut num_results = 0;
        for (num_tasks, task) in tasks.values().enumerate() {
            println!("export TASK{}='{}'", num_tasks, serde_json::to_string(task).unwrap().replace('\'', "\'"));
            for result in task.results.values() {
                println!("export RESULT{}='{}'", num_results, serde_json::to_string(result).unwrap().replace('\'', "\'"));
                num_results += 1;
            }
        }
        true
    } else {
        false
    }
}

pub fn generate_example_tasks(broker: Option<BrokerId>, proxy: Option<ProxyId>) -> HashMap<MsgId, MsgTaskRequest> {
    let mut tasks: HashMap<MsgId, MsgTaskRequest> = HashMap::new();
    let broker = broker.unwrap_or_else(|| BrokerId::new(&config::CONFIG_SHARED.broker_domain).unwrap());
    let proxy = {
        if let Some(id) = proxy {
            if ! id.can_be_signed_by(&broker) {
                panic!("Submitted proxy_id ({id}) cannot be signed by submitted broker_id ({broker})");
            }
            id
        } else {
            ProxyId::new(&format!("proxy{}.{}", random_number(), broker)).unwrap()
        }
    };
    let app1 = AppId::new(&format!("app1.{proxy}")).unwrap();
    let app2 = AppId::new(&format!("app2.{proxy}")).unwrap();

    let task_for_apps_1_2 = MsgTaskRequest::new(
        app1.clone().into(),
        vec![app1.clone().into(), app2.clone().into()],
        "My important task".to_string(),
        "This task is for app1 and app2".to_string(),
        FailureStrategy::Retry { backoff_millisecs: 1000, max_tries: 5 },
    );

    let response_by_app1 = MsgTaskResult {
        id: MsgId::new(),
        from: app1.clone().into(),
        to: vec![app1.clone().into()],
        task: task_for_apps_1_2.id,
        result: crate::WorkResult::Succeeded("All done!".to_string()),
    };
    let response_by_app2 = MsgTaskResult {
        id: MsgId::new(),
        from: app2.into(),
        to: vec![app1.into()],
        task: task_for_apps_1_2.id,
        result: crate::WorkResult::PermFailed("Unable to complete".to_string()),
    };
    tasks.insert(task_for_apps_1_2.id, task_for_apps_1_2);
    let task_in_map = tasks.values_mut().next().unwrap(); // only used in testing
    for result in [response_by_app1, response_by_app2] {
        let result = MsgSigned{
            msg: result,
            sig: String::from("just_an_example"),
        };
        task_in_map.results.insert(result.msg.from.clone(), result);
    }
    tasks
}

// Random

impl AppId {
    pub fn random(parent: &ProxyId) -> Self {
        let mut rnd = random_str();
        rnd.push('.');
        rnd.push_str(&parent.to_string());
        Self::new(&rnd)
            .expect("Internal error: Tried to construct faulty random AppId")
    }
}

impl ProxyId {
    pub fn random(parent: &BrokerId) -> Self {
        let mut rnd = random_str();
        rnd.push('.');
        rnd.push_str(&parent.to_string());
        Self::new(&rnd)
            .expect("Internal error: Tried to construct faulty random ProxyId")
    }
}

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