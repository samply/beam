use std::time::{SystemTime, Duration};

use axum::{Router, routing::{get, post}, response::{Response, IntoResponse}, extract::{State, Path}};
use hyper::{Request, Body, StatusCode, upgrade::OnUpgrade};
use shared::{http_client::SamplyHttpClient, config, MsgSocketRequest, beam_id::AppOrProxyId, MsgId, Plain, MsgEmpty};
use tracing::warn;

use crate::{serve_tasks::{handler_task, TasksState, forward_request}, auth::AuthenticatedApp};


pub(crate) fn router(client: SamplyHttpClient) -> Router {
    let config = config::CONFIG_PROXY.clone();
    let state = TasksState {
        client: client.clone(),
        config,
    };
    Router::new()
        .route("/v1/sockets", get(handler_task).post(handler_task))
        .route("/v1/sockets/:app_or_id", post(create_socket_con).get(connect_socket))
        // .route("/v1/sockets/:id/results", get(handler_task).put(handler_task))
        .with_state(state)
}

async fn create_socket_con(
    AuthenticatedApp(sender): AuthenticatedApp,
    Path(to): Path<AppOrProxyId>,
    state: State<TasksState>,
    req: Request<Body>
) -> Response {
    let task_id = MsgId::new();
    // TODO: proper secrets and encryption
    let secret = "";
    let socket_req = MsgSocketRequest {
        from: AppOrProxyId::AppId(sender.clone()),
        to: vec![to],
        expire: SystemTime::now() + Duration::from_secs(60),
        id: task_id,
        secret: Plain::from(secret),
        metadata: serde_json::Value::Null,
        result: Vec::with_capacity(0),
    };

    let Ok(body) = serde_json::to_vec(&socket_req) else {
        warn!("Failed to serialize MsgSocketRequest");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let new_req = Request::post("/v1/sockets")
        .body(Body::from(body));
    let post_socket_task_req = match new_req {
        Ok(req) => req,
        Err(e) => {
            warn!("Failed to construct request: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        },
    };

    let res = match forward_request(post_socket_task_req, &state.config, &sender, &state.client).await {
        Ok(res) => res,
        Err(err) => {
            warn!("Failed to create post socket request: {err:?}");
            return err.into_response()
        },
    };

    if res.status() != StatusCode::CREATED {
        warn!("Failed to post MsgSocketRequest to broker. Statuscode: {}", res.status());
        return (res.status(), "Failed to post MsgSocketRequest to broker").into_response();
    }
    connect_socket(AuthenticatedApp(sender), state, task_id, req).await
}

async fn connect_socket(
    AuthenticatedApp(sender): AuthenticatedApp,
    state: State<TasksState>,
    task_id: MsgId,
    req: Request<Body>
) -> Response {
    if req.extensions().get::<OnUpgrade>().is_none() {
        return StatusCode::UPGRADE_REQUIRED.into_response();
    }

    let msg_empty = MsgEmpty {
        from: AppOrProxyId::AppId(sender.clone()),
    };
    let Ok(body) = serde_json::to_vec(&msg_empty) else {
        warn!("Failed to serialize MsgEmpty");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    // Try to connect to socket
    let new_req = Request::get(format!("/v1/sockets/{task_id}")).body(Body::from(body));
    let mut get_socket_con_req = match new_req {
        Ok(req) => req,
        Err(e) => {
            warn!("Failed to construct request: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        },
    };
    *get_socket_con_req.headers_mut() = req.headers().clone();

    let res = match forward_request(get_socket_con_req, &state.config, &sender, &state.client).await {
        Ok(res) => res,
        Err(err) => {
            warn!("Failed to create socket connect request: {err:?}");
            return err.into_response()
        },
    };

    if res.extensions().get::<OnUpgrade>().is_none() || res.status() != StatusCode::SWITCHING_PROTOCOLS {
        warn!("Failed to create an upgradable connection to the broker. Response was: {res:?}");
        return res.status().into_response();
    }

    // Connect sockets
    tokio::spawn(async move {
        // TODO: Encrypt
        let mut c1 = hyper::upgrade::on(res).await.unwrap();
        let mut c2 = hyper::upgrade::on(req).await.unwrap();

        let result = tokio::io::copy_bidirectional(&mut c1, &mut c2).await;
        if let Err(e) = result {
            warn!("Error relaying socket connect: {e}");
        }
    });

    StatusCode::SWITCHING_PROTOCOLS.into_response()
}
