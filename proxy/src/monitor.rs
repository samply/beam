use std::{
    convert::Infallible,
    net::SocketAddr,
    sync::atomic::{AtomicBool, Ordering},
};

use axum::{
    async_trait,
    body::Bytes,
    extract::{ConnectInfo, Path, State},
    http::request,
    middleware::Next,
    response::{sse::Event, IntoResponse, Response, Sse},
    routing::get,
    Router,
};
use hyper::{body::Buf, Body, HeaderMap, Method, Request, StatusCode, Uri};
use serde::{Serialize, Serializer};
use serde_json::Value;
use shared::once_cell::sync::Lazy;
use shared::{MsgId, MsgTaskRequest};
use tokio::sync::broadcast::{self, Receiver, Sender};
use tower_http::services::{ServeDir, ServeFile};
use tracing::{error, info};

/// If monitoring is active this function will await the full request and response bodys
pub async fn monitor_and_log(
    s: ConnectInfo<SocketAddr>,
    mut req: Request<Body>,
    next: Next<Body>,
) -> Response {
    // TODO combine req and response into single type
    MONITORER.send(&mut req);
    // TODO put this where it was and just layer this middleware
    let mut resp = shared::middleware::log(s, req, next).await;
    MONITORER.send(&mut resp);
    resp
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

pub trait IntoMonitoringUpdate {
    fn into_monitoring(self) -> Option<MonitoringUpdate>;
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
    pub fn send(&self, update: impl IntoMonitoringUpdate) {
        if !self.should_record.load(Ordering::Relaxed) {
            return;
        }
        if let Some(update) = update.into_monitoring() {
            if self.task_sender.send(update).is_err() {
                info!("Noone is listening");
                MONITORER.stop_recording();
            };
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub enum MonitoringUpdate {
    Request {
        #[serde(with = "hyper_serde")]
        uri: Uri,
        #[serde(with = "hyper_serde")]
        method: Method,
        #[serde(with = "hyper_serde")]
        headers: HeaderMap,
    },
    Response {
        #[serde(with = "hyper_serde")]
        status: StatusCode,
        #[serde(with = "hyper_serde")]
        headers: HeaderMap,
    },
}

impl MonitoringUpdate {
    fn into_event(self) -> Option<Event> {
        Event::default().json_data(self).ok()
    }
}

impl IntoMonitoringUpdate for &mut Request<Body> {
    fn into_monitoring(self) -> Option<MonitoringUpdate> {
        Some(MonitoringUpdate::Request {
            method: self.method().to_owned(),
            headers: self.headers().to_owned(),
            uri: self.uri().to_owned(),
        })
    }
}

impl IntoMonitoringUpdate for &mut Response {
    fn into_monitoring(self) -> Option<MonitoringUpdate> {
        Some(MonitoringUpdate::Response {
            status: self.status(),
            headers: self.headers().to_owned(),
        })
    }
}
