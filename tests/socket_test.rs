
use std::{time::{SystemTime, Duration}, future::Future};

use http::{Request, header, Method, Response, StatusCode};
use hyper::{Body, upgrade::Upgraded};
use rand::RngCore;
use shared::{http_client::SamplyHttpClient, MsgSocketRequest, Plain, MsgId, MsgEmpty, beam_id::AppOrProxyId};
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use serde_json::Value;
use anyhow::Result;
use tests::*;

async fn upgrade(res: impl Future<Output = Response<Body>>) -> Upgraded {
    hyper::upgrade::on(res.await).await.expect("Upgrade successfull")
}

async fn test_connections(r1: impl Future<Output = Response<Body>>, r2: impl Future<Output = Response<Body>>) {
    let (mut a, mut b) = tokio::join!(
        upgrade(r1),
        upgrade(r2)
    );
    const N: usize = 2_usize.pow(13);
    let test_data: &mut [u8; N] = &mut [0; N];
    rand::thread_rng().fill_bytes(test_data);
    let mut read_buf = [0; N];
    a.write_all(test_data).await.unwrap();
    a.flush().await.unwrap();
    b.read_exact(&mut read_buf).await.unwrap();
    assert_eq!(test_data, &read_buf);
}

#[tokio::test]
async fn test_full() -> Result<()> {
    let client = shared::http_client::build(&Vec::new(), None, None)?;
    let client2 = client.clone();

    // let res = post_socket_req(client.clone(), task_id).await?;
    // assert_eq!(res.status(), StatusCode::CREATED);
    let app1 = async {
        let res = create_connect_socket(client, &APP2).await.expect("Failed to create socket connection");
        assert_eq!(res.status(), StatusCode::SWITCHING_PROTOCOLS);
        res
    };
    let app2 = async {
        let mut res = get_task(client2.clone()).await.expect("Getting task failed");
        assert_eq!(res.status(), StatusCode::OK);
        let body = hyper::body::to_bytes(res.body_mut()).await.expect("Failed to read body");
        let tasks: Vec<MsgSocketRequest<Plain>> = serde_json::from_slice(&body).expect("Failed to deserialize body to socket reqs");
        assert_eq!(tasks.len(), 1);
        let res = connect_socket(client2, &tasks[0].id).await.expect("Failed to create socket connection");
        assert_eq!(res.status(), StatusCode::SWITCHING_PROTOCOLS);
        res
    };


    // let res = put_socket_result(client.clone(), task_id).await?;
    // assert_eq!(res.status(), StatusCode::CREATED);

    // let res = get_task_result(client.clone(), task_id).await?;
    // assert_eq!(res.status(), StatusCode::OK);
    test_connections(app1, app2).await;
    Ok(())
}

async fn create_connect_socket(client: SamplyHttpClient, app: &AppOrProxyId) -> Result<Response<Body>> {
    let req = Request::builder()
        .method(Method::POST)
        .uri(format!("{PROXY1}/v1/sockets/{app}"))
        .header(header::AUTHORIZATION, format!("ApiKey {} {APP_KEY}", APP1.clone()))
        .header(header::UPGRADE, "tcp")
        .body(Body::empty())?;
    Ok(client.request(req).await?)
}

async fn connect_socket(client: SamplyHttpClient, task_id: &MsgId) -> Result<Response<Body>> {
    let req = Request::builder()
        .method(Method::GET)
        .uri(format!("{PROXY2}/v1/sockets/{task_id}"))
        .header(header::AUTHORIZATION, format!("ApiKey {} {APP_KEY}", APP2.clone()))
        .header(header::UPGRADE, "tcp")
        .body(Body::empty())?;
    Ok(client.request(req).await?)
}

async fn post_socket_req(client: SamplyHttpClient, task_id: &MsgId) -> Result<Response<Body>> {
    let task = MsgSocketRequest {
        from: APP1.clone(),
        to: vec![APP2.clone()],
        expire: SystemTime::now() + Duration::from_secs(60),
        id: *task_id,
        secret: Plain::from("test"),
        metadata: Value::Null,
    };
    let req = Request::builder()
        .uri(format!("{PROXY1}/v1/sockets"))
        .header(header::AUTHORIZATION, format!("ApiKey {} {APP_KEY}", APP1.clone()))
        .method(Method::POST)
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
