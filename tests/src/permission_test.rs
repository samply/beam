use std::time::{SystemTime, Duration};

use http::{Request, StatusCode, Method};
use shared::{MsgId, Plain, beam_id::{AppOrProxyId, BeamId}};

use crate::{BeamRequestBuilder, APP1, APP_KEY, CLIENT, PROXY1};


#[tokio::test]
async fn test_no_senders() {
    let req = Request::builder()
        .uri(format!("{PROXY1}/v1/tasks"))
        .method(Method::POST)
        .as_app(&APP1, APP_KEY)
        .with_json(&shared::MsgTaskRequest {
            id: MsgId::new(),
            from: APP1.clone(),
            to: vec![AppOrProxyId::new("app1.proxy3.broker").unwrap()],
            body: Plain::from(""),
            expire: SystemTime::now() + Duration::from_secs(60),
            failure_strategy: shared::FailureStrategy::Discard,
            results: Default::default(),
            metadata: serde_json::Value::Null
        })
        .unwrap();
    let res = CLIENT.request(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY)
}

#[tokio::test]
async fn test_allowed_sender() {
    let req = Request::builder()
        .uri(format!("{PROXY1}/v1/tasks"))
        .method(Method::POST)
        .as_app(&APP1, APP_KEY)
        .with_json(&shared::MsgTaskRequest {
            id: MsgId::new(),
            from: APP1.clone(),
            to: vec![AppOrProxyId::new("app2.proxy3.broker").unwrap()],
            body: Plain::from(""),
            expire: SystemTime::now() + Duration::from_secs(60),
            failure_strategy: shared::FailureStrategy::Discard,
            results: Default::default(),
            metadata: serde_json::Value::Null
        })
        .unwrap();
    let res = CLIENT.request(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED)
}
