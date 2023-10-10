
use http::{Request, header, StatusCode};
use hyper::{Client, Body};
use once_cell::sync::Lazy;
use beam_lib::{AddressingId, set_broker_id, AppOrProxyId, BeamClient};

#[cfg(all(feature = "sockets", test))]
mod socket_test;

#[cfg(test)]
mod task_test;

#[cfg(test)]
mod test_sse;

pub static APP1: Lazy<AddressingId> = Lazy::new(|| {
    set_broker_id("broker".into());
    AppOrProxyId::new(option_env!("APP1_P1").unwrap_or("app1.proxy1.broker")).unwrap()
}); 
pub static APP2: Lazy<AddressingId> = Lazy::new(|| {
    set_broker_id("broker".into());
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

pub static CLIENT1: Lazy<BeamClient> = Lazy::new(|| BeamClient::new(&APP1, APP_KEY, PROXY1.parse().unwrap()));
pub static CLIENT2: Lazy<BeamClient> = Lazy::new(|| BeamClient::new(&APP2, APP_KEY, PROXY2.parse().unwrap()));

#[tokio::test]
async fn test_time_out() {
    let res = Client::new().request(Request::get("http://localhost:8081/v1/tasks?wait_count=100&filter=todo&wait_time=5s")
        .header(header::AUTHORIZATION, format!("ApiKey {} {APP_KEY}", APP1.clone()))
        .body(Body::empty()).unwrap()
    ).await.unwrap();

    assert_eq!(res.status(), StatusCode::PARTIAL_CONTENT);
}
