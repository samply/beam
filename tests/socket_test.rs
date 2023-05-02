
use std::time::{SystemTime, Duration};

use fast_socks5::client::{Config, Socks5Stream};
use http::{Request, HeaderName, header, method, Method, Response, StatusCode};
use hyper::Body;
use shared::{http_client::SamplyHttpClient, MsgSocketRequest, beam_id::{AppOrProxyId, AppId, BeamId}, Plain, MyUuid};
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use serde_json::Value;
use anyhow::Result;
use tests::*;

#[tokio::test]
async fn test_sockets() {
    let (a, b) = tokio::join!(
        Socks5Stream::connect_with_password("localhost:8090", "We dont care".to_string(), 1337, "test1".to_string(), "test".to_string(), Config::default()),
        Socks5Stream::connect_with_password("localhost:8090", "We dont care".to_string(), 1337, "test2".to_string(), "test".to_string(), Config::default())
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

#[tokio::test]
async fn test_full() -> Result<()> {
    let client = shared::http_client::build(&Vec::new(), None, None)?;
    let mut res = post_socket_req(client).await?;
    let body = hyper::body::to_bytes(res.body_mut()).await?;
    println!("response: {}", String::from_utf8(body.to_vec())?);
    assert_eq!(res.status(), StatusCode::CREATED);
    Ok(())
}

async fn post_socket_req(client: SamplyHttpClient) -> Result<Response<Body>> {
    let task = MsgSocketRequest {
        from: APP1.clone(),
        to: APP2.clone(),
        expire: SystemTime::now() + Duration::from_secs(60),
        id: MyUuid::new(),
        secret: Plain::from("test"),
        metadata: Value::Null,
        result: None,
    };
    let req = Request::builder()
        .uri(format!("{PROXY1}/v1/sockets"))
        .header(header::AUTHORIZATION, format!("ApiKey {} {APP_KEY}", APP1.clone()))
        .method(Method::POST)
        .body(hyper::Body::from(serde_json::to_vec(&task)?))?;
    let resp = client.request(req).await?;
    Ok(resp)
}
