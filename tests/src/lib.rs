
use hyper::{Body, Client, client::HttpConnector};
use http::{Request, header, request, StatusCode};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use shared::{beam_id::{AppOrProxyId, BeamId, AppId}, MsgId, MsgTaskRequest, Plain, FailureStrategy};

#[cfg(all(feature = "sockets", test))]
mod socket_test;

#[cfg(test)]
mod permission_test;

pub static APP1: Lazy<AppOrProxyId> = Lazy::new(|| {
    AppId::set_broker_id("broker".to_string());
    AppOrProxyId::new(option_env!("APP1_P1").unwrap_or("app1.proxy1.broker")).unwrap()
}); 
pub static APP2: Lazy<AppOrProxyId> = Lazy::new(|| {
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


pub fn beam_request(r#as: &AppOrProxyId, path: &str) -> request::Builder {
    let proxy = match r#as {
        app if app == &*APP1 => PROXY1,
        app if app == &*APP2 => PROXY2,
        _ => panic!("Failed to find matching proxy for app")
    };
    Request::builder()
        .as_app(r#as, APP_KEY)
        .uri(format!("{proxy}{path}"))
}

pub static CLIENT: Lazy<Client<HttpConnector, Body>> = Lazy::new(|| Client::new());

// This could be in a beam lib as well maybe
trait BeamRequestBuilder {
    fn as_app(self, app: &AppOrProxyId, key: &str) -> Self;
    // We do a generic B here to not require hyper as a dependency
    fn with_json<B: From<Vec<u8>>, T: Serialize>(self, json: &T) -> Result<Request<B>, http::Error>;
}

impl BeamRequestBuilder for request::Builder {
    fn as_app(self, app: &AppOrProxyId, key: &str) -> Self {
        self.header(header::AUTHORIZATION, format!("ApiKey {app} {key}"))
    }

    fn with_json<B: From<Vec<u8>>, T: Serialize>(self, json: &T) -> Result<Request<B>, http::Error> {
        self.body(B::from(serde_json::to_vec(json).unwrap()))
    }
}

// Move to beam lib when I get to write it
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
