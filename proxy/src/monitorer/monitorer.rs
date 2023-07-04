use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::atomic::{AtomicBool, Ordering},
};

use axum::{
    async_trait,
    body::{boxed, Bytes, Full},
    extract::{ConnectInfo, Path, State},
    http::{request, response, HeaderName},
    middleware::Next,
    response::{sse::Event, IntoResponse, Response, Sse},
    routing::get,
    Router, TypedHeader, headers::Cookie,
};
use hyper::{body::Buf, header, Body, HeaderMap, Method, Request, StatusCode, Uri};
use once_cell::sync::Lazy;
use serde::{Serialize, Serializer};
use serde_json::Value;
use shared::{MsgId, MsgTaskRequest, config::CONFIG_PROXY, PlainMessage};
use tokio::sync::broadcast::{self, Receiver, Sender};
use tracing::{error, debug};

pub fn router() -> Router {
    Router::new()
        .route("/v1/monitor/events", get(stream_recorded_tasks))
}

pub async fn stream_recorded_tasks(cookies: TypedHeader<Cookie>) -> Response {
    if !cookies
        .get("monitoring_key")
        .map(|value| value == CONFIG_PROXY.monitoring_api_key)
        .unwrap_or(false) 
    {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    MONITORER.start_recording();
    let mut receiver = MONITORER.get_receiver();
    let task_stream = async_stream::stream! {
        while let Ok(update) = receiver.recv().await {
            // Maybe return some actual error when all senders are dropped
            if let Some(event) = update.into_event() {
                yield Result::<_, Infallible>::Ok(event)
            }
        }
        // I think this will never happen
        error!("All senders have been dropped or receiver is lagging. connection closed");
    };
    Sse::new(task_stream).into_response()
}

pub struct Monitorer {
    should_record: AtomicBool,
    task_sender: Sender<MonitoringUpdate>,
}

impl Default for Monitorer {
    fn default() -> Self {
        let (tx, _) = broadcast::channel(32);
        Self {
            should_record: Default::default(),
            task_sender: tx,
        }
    }
}

pub(crate) static MONITORER: Lazy<Monitorer> = Lazy::new(|| Monitorer::default());

impl Monitorer {
    fn get_receiver(&self) -> Receiver<MonitoringUpdate> {
        self.task_sender.subscribe()
    }

    fn start_recording(&self) {
        self.should_record.store(true, Ordering::Relaxed);
    }

    fn stop_recording(&self) {
        self.should_record.store(false, Ordering::Relaxed);
    }

    /// Sends an update to the monitorer
    /// Returns the number of listeners
    pub fn send(&self, update: impl Into<MonitoringUpdate>) {
        if !self.should_record.load(Ordering::Relaxed) {
            return;
        }
        if self.task_sender.send(update.into()).is_err() {
            debug!("Noone is listening");
            self.stop_recording();
        };
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MonitoringUpdate {
    /// Request from a proxy internal app to the broker
    Request {
        #[serde(with = "hyper_serde")]
        uri: Uri,
        #[serde(with = "hyper_serde")]
        method: Method,
        #[serde(with = "hyper_serde")]
        headers: HeaderMap,
        json: Value,
    },
    /// Response from the broker to a proxy internal app
    Response {
        #[serde(with = "hyper_serde")]
        status: StatusCode,
        #[serde(with = "hyper_serde")]
        headers: HeaderMap,
        json: Value,
    },
}

impl MonitoringUpdate {
    fn into_event(self) -> Option<Event> {
        Event::default().json_data(self).ok()
    }
}

impl From<(&request::Parts, &PlainMessage)> for MonitoringUpdate {
    fn from((parts, json): (&request::Parts, &PlainMessage)) -> Self {
        MonitoringUpdate::Request {
            method: parts.method.to_owned(),
            headers: parts.headers.to_owned(),
            uri: parts.uri.to_owned(),
            json: serde_json::to_value(json).expect("Plain message should serialize"),
        }
    }
}

impl From<(&response::Parts, &Value)> for MonitoringUpdate {
    fn from((parts, json): (&response::Parts, &Value)) -> Self {
        MonitoringUpdate::Response {
            status: parts.status,
            headers: parts.headers.to_owned(),
            json: json.to_owned(),
        }
    }
}
