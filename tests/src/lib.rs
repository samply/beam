
use std::time::{SystemTime, Duration};

use http::{Request, header, StatusCode};
use hyper::{Client, Body};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use shared::{beam_id::{AppOrProxyId, BeamId, AppId}, MsgId, MsgTaskRequest, Plain, FailureStrategy};

#[cfg(all(feature = "sockets", test))]
mod socket_test;

pub const APP1: Lazy<AppOrProxyId> = Lazy::new(|| {
    AppId::set_broker_id("broker".to_string());
    AppOrProxyId::new(option_env!("APP1_P1").unwrap_or("app1.proxy1.broker")).unwrap()
}); 
pub const APP2: Lazy<AppOrProxyId> = Lazy::new(|| {
    AppId::set_broker_id("broker".to_string());
    AppOrProxyId::new(option_env!("APP2_P2").unwrap_or("app2.proxy2.broker")).unwrap()
}); 

// unwrap_or is not const yet
pub const PROXY1: &str = match option_env!("P1") {
    Some(v) => v,
    _ => "http://localhost:8081"
};
pub const PROXY2: &str = match option_env!("P2") {
    Some(v) => v,
    _ => "http://localhost:8082"
};

pub const APP_KEY: &str = match option_env!("APP_KEY") {
    Some(v) => v,
    None => "App1Secret"
};


#[derive(Debug, Serialize, Deserialize)]
pub struct SocketTask {
    pub to: Vec<AppOrProxyId>,
    pub from: AppOrProxyId,
    pub ttl: String,
    pub id: MsgId,
}

#[tokio::test]
async fn test_time_out() {
    let res = Client::new().request(Request::get("http://localhost:8081/v1/tasks?wait_count=100&filter=todo&wait_time=5s")
        .header(header::AUTHORIZATION, format!("ApiKey {} {APP_KEY}", APP1.clone()))
        .body(Body::empty()).unwrap()
    ).await.unwrap();

    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
}
