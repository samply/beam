
use std::time::{SystemTime, Duration};

use fast_socks5::client::{Config, Socks5Stream};
use http::{Request, header, Method, Response, StatusCode};
use hyper::Body;
use shared::{http_client::SamplyHttpClient, MsgSocketRequest, Plain, MyUuid, MsgSocketResult, MsgId, MsgEmpty};
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use serde_json::Value;
use anyhow::Result;
use tests::*;

async fn test_sockets(secret: String) {
    let (a, b) = tokio::join!(
        Socks5Stream::connect_with_password("localhost:8090", "We dont care".to_string(), 1337, "test1".to_string(), secret.clone(), Config::default()),
        Socks5Stream::connect_with_password("localhost:8090", "We dont care".to_string(), 1337, "test2".to_string(), secret, Config::default())
    );
    let mut a = a.unwrap();
    let mut b = b.unwrap();
    const TEST_DATA: &[u8; 11] = b"hello world";
    a.write_all(TEST_DATA).await.unwrap();
    a.flush().await.unwrap();
    let recieved = &mut [0 as u8; TEST_DATA.len()];
    b.read_exact(recieved).await.unwrap();
    assert_eq!(TEST_DATA, recieved);
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn test_sockets_default() {
    test_sockets("test".to_string()).await;
}

#[tokio::test]
async fn test_full() -> Result<()> {
    let client = shared::http_client::build(&Vec::new(), None, None)?;
    let task_id = &MyUuid::new();

    let res = post_socket_req(client.clone(), task_id).await?;
    assert_eq!(res.status(), StatusCode::CREATED);

    let res = get_task(client.clone()).await?;
    assert_eq!(res.status(), StatusCode::OK);

    let res = put_socket_result(client.clone(), task_id).await?;
    assert_eq!(res.status(), StatusCode::CREATED);
    println!("{:?}", res.headers().get(header::LOCATION));

    let res = get_task_result(client.clone(), task_id).await?;
    assert_eq!(res.status(), StatusCode::OK);
    println!("{:?}", res.headers().get(header::LOCATION));

    test_sockets("hashofsecret".to_string()).await;
    Ok(())
}

async fn post_socket_req(client: SamplyHttpClient, task_id: &MsgId) -> Result<Response<Body>> {
    let task = MsgSocketRequest {
        from: APP1.clone(),
        to: vec![APP2.clone()],
        expire: SystemTime::now() + Duration::from_secs(60),
        id: *task_id,
        secret: Plain::from("test"),
        metadata: Value::Null,
        result: vec![],
    };
    let req = Request::builder()
        .uri(format!("{PROXY1}/v1/sockets"))
        .header(header::AUTHORIZATION, format!("ApiKey {} {APP_KEY}", APP1.clone()))
        .method(Method::POST)
        .body(hyper::Body::from(serde_json::to_vec(&task)?))?;
    let resp = client.request(req).await?;
    Ok(resp)
}

async fn put_socket_result(client: SamplyHttpClient, task_id: &MsgId) -> Result<Response<Body>> {
    let task = MsgSocketResult {
        from: APP2.clone(),
        to: vec![APP1.clone()],
        task: *task_id,
        metadata: Value::Null,
        token: "hashofsecret".to_string(),
    };
    let req = Request::builder()
        .uri(format!("{PROXY2}/v1/sockets/{task_id}/results"))
        .header(header::AUTHORIZATION, format!("ApiKey {} {APP_KEY}", APP2.clone()))
        .method(Method::PUT)
        .body(hyper::Body::from(serde_json::to_vec(&task)?))?;
    let resp = client.request(req).await?;
    Ok(resp)
}

async fn get_task(client: SamplyHttpClient) -> Result<Response<Body>> {
    let msg = MsgEmpty {
        from: APP2.clone(),
    };
    let req = Request::builder()
        .uri(format!("{PROXY2}/v1/sockets?wait_count=1"))
        .header(header::AUTHORIZATION, format!("ApiKey {} {APP_KEY}", APP2.clone()))
        .method(Method::GET)
        .body(hyper::Body::from(serde_json::to_vec(&msg)?))?;
    let resp = client.request(req).await?;
    Ok(resp)
}

async fn get_task_result(client: SamplyHttpClient, task_id: &MsgId) -> Result<Response<Body>> {
    let msg = MsgEmpty {
        from: APP1.clone(),
    };
    let req = Request::builder()
        .uri(format!("{PROXY1}/v1/sockets/{task_id}/results"))
        .header(header::AUTHORIZATION, format!("ApiKey {} {APP_KEY}", APP1.clone()))
        .method(Method::GET)
        .body(hyper::Body::from(serde_json::to_vec(&msg)?))?;
    let resp = client.request(req).await?;
    Ok(resp)
}
