
use std::{sync::{atomic::{AtomicBool, Ordering}, Arc}, convert::Infallible};

use axum::{extract::{State, Path}, response::{Response, Sse, IntoResponse, sse::Event}, Router, routing::get};
use hyper::{Request, Body, Method};
use shared::once_cell::sync::Lazy;
use serde_json::Value;
use shared::{MsgId, MsgTaskRequest};
use tokio::sync::broadcast::{Sender, self, Receiver};
use tracing::{info, error};
use tower_http::services::{ServeDir, ServeFile};


pub fn router() -> Router {
    let servic = ServeDir::new("./monitorer/dist").not_found_service(ServeFile::new("./monitorer/dist/index.html"));
    Router::new()
        .route("/api/events", get(stream_recorded_tasks))
        .fallback_service(servic)
}

// TODO this needs some form of Auth
pub async fn stream_recorded_tasks() -> impl IntoResponse {
    MONITORER.start_recording();
    let task_stream = async_stream::stream! {
        let mut receiver = MONITORER.get_receiver();
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
        Self { should_record: Default::default(), task_sender: tx }
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
    pub fn send(&self, update: impl TryInto<MonitoringUpdate>) {
        if !self.should_record.load(Ordering::Relaxed) {
            return;
        }
        if let Ok(update) = update.try_into() {
            if self.task_sender.send(update).is_err() {
                info!("Noone is listening");
                MONITORER.stop_recording();
            };
        }
    }
}

#[derive(Debug, Clone)]
pub enum MonitoringUpdate {
    
}

impl MonitoringUpdate {
    fn into_event(self) -> Option<Event> {
        todo!()
    }
}
