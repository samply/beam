use std::{
    fmt::Debug,
    time::{Duration, SystemTime},
};

use beam_lib::{set_broker_id, AppOrProxyId, MsgId};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::json;

use crate::Plain;

fn assert_json_eq<A, B>(a: A, b: B)
where
    A: Serialize + DeserializeOwned + Debug,
    B: Serialize + DeserializeOwned + Debug,
{
    let a_str = serde_json::to_string(&a).unwrap();
    let b_str = serde_json::to_string(&b).unwrap();
    assert_eq!(a_str, b_str);
    assert!(serde_json::from_str::<A>(&b_str).is_ok());
    assert!(serde_json::from_str::<B>(&a_str).is_ok());
}

#[test]
fn test_msg_empty() {
    set_broker_id("broker.samply.de".to_string());
    let internal = crate::MsgEmpty {
        from: AppOrProxyId::new("app1.proxy1.broker.samply.de").unwrap(),
    };
    let lib = beam_lib::MsgEmpty {
        from: AppOrProxyId::new("app1.proxy1.broker.samply.de").unwrap(),
    };
    assert_json_eq(internal, lib);
}

#[test]
fn test_msg_task() {
    set_broker_id("broker.samply.de".to_string());
    let json_data = json!({
        "foo": 1,
        "bar": true,
    });
    let id = MsgId::new();
    let internal = crate::MsgTaskRequest {
        from: AppOrProxyId::new("app1.proxy1.broker.samply.de").unwrap(),
        to: vec![],
        id,
        body: Plain::from(serde_json::to_string(&json_data).unwrap()),
        expire: SystemTime::now() + Duration::from_secs(10),
        failure_strategy: crate::FailureStrategy::Retry {
            backoff_millisecs: 100,
            max_tries: 10,
        },
        results: Default::default(),
        metadata: json_data.clone(),
    };
    let lib = beam_lib::TaskRequest {
        from: AppOrProxyId::new("app1.proxy1.broker.samply.de").unwrap(),
        id,
        to: vec![],
        body: json_data.clone(),
        ttl: "9".to_string(),
        failure_strategy: beam_lib::FailureStrategy::Retry {
            backoff_millisecs: 100,
            max_tries: 10,
        },
        metadata: json_data,
    };
    assert_json_eq(lib, internal);
}

#[test]
fn test_task_result() {
    set_broker_id("broker.samply.de".to_string());
    let json_data = json!({
        "foo": 1,
        "bar": true,
    });
    let task = MsgId::new();
    let from = AppOrProxyId::new("app1.proxy1.broker.samply.de").unwrap();
    let internal = crate::MsgTaskResult {
        from: from.clone(),
        to: vec![],
        body: Plain::from(serde_json::to_string(&json_data).unwrap()),
        metadata: json_data.clone(),
        task,
        status: crate::WorkStatus::Succeeded,
    };
    let lib = beam_lib::TaskResult {
        from,
        to: vec![],
        task,
        status: beam_lib::WorkStatus::Succeeded,
        body: json_data.clone(),
        metadata: json_data,
    };
    assert_json_eq(lib, internal);
}

#[cfg(feature = "sockets")]
#[test]
fn test_socket_task() {
    set_broker_id("broker.samply.de".to_string());
    let id = MsgId::new();
    let from = AppOrProxyId::new("app1.proxy1.broker.samply.de").unwrap();
    let internal = crate::MsgSocketRequest {
        from: from.clone(),
        to: vec![],
        secret: Plain { body: None },
        expire: SystemTime::now() + Duration::from_secs(10),
        id,
        metadata: serde_json::Value::Null
    };
    let lib = beam_lib::SocketTask {
        from,
        to: vec![],
        ttl: "9".to_string(),
        id,
        metadata: serde_json::Value::Null
    };
    let a_str = serde_json::to_string(&lib).unwrap();
    let b_str = serde_json::to_string(&internal).unwrap();
    assert_eq!(a_str, b_str);
    assert!(serde_json::from_str::<beam_lib::SocketTask>(&b_str).is_ok());
}
