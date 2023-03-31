use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::atomic::{AtomicBool, Ordering},
};

use axum::{
    async_trait,
    body::Bytes,
    extract::{ConnectInfo, Path, State},
    http::{request, response},
    middleware::Next,
    response::{sse::Event, IntoResponse, Response, Sse},
    routing::get,
    Router,
};
use hyper::{body::Buf, Body, HeaderMap, Method, Request, StatusCode, Uri};
use serde::{Serialize, Serializer};
use serde_json::Value;
use shared::{once_cell::sync::Lazy, PlainMessage};
use shared::{MsgId, MsgTaskRequest};
use tokio::sync::broadcast::{self, Receiver, Sender};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{error, info};

pub async fn monitor(s: ConnectInfo<SocketAddr>, req: Request<Body>, next: Next<Body>) -> Response {
    // Maybe use this to log everthing
    todo!()
}

pub fn router() -> Router {
    let servic = ServeDir::new("./dist").not_found_service(ServeFile::new("./dist/index.html"));
    Router::new()
        .route("/monitor/events", get(stream_recorded_tasks))
        .fallback_service(servic)
}

// TODO this needs some form of Auth
pub async fn stream_recorded_tasks() -> impl IntoResponse {
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
        error!("All senders have been droped or reciever is lagging. connection closed");
    };
    Sse::new(task_stream)
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

pub static MONITORER: Lazy<Monitorer> = Lazy::new(|| Monitorer::default());

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
            info!("Noone is listening");
            MONITORER.stop_recording();
        };
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
// TODO: Make this internally tagged as ts is better at understading it
pub enum MonitoringUpdate {
    Request {
        #[serde(with = "hyper_serde")]
        uri: Uri,
        #[serde(with = "hyper_serde")]
        method: Method,
        #[serde(with = "hyper_serde")]
        headers: HeaderMap,
        json: PlainMessage,
    },
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
            json: json.to_owned(),
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
