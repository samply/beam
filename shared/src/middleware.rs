use std::{
    cell::RefCell,
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

use axum::{
    body::HttpBody,
    extract::{ConnectInfo, Request},
    http::{header::HeaderName, Method, StatusCode, Uri},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use tokio::sync::mpsc;
use tracing::{error, info, instrument, span, warn, Level};

use beam_lib::AppOrProxyId;

const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

pub struct LoggingInfo {
    // Known from the start
    method: Method,
    uri: Uri,
    ip: IpAddr,

    // Added by SignedMsg extractor
    from_proxy: Option<AppOrProxyId>,

    // Added after handlers
    status_code: Option<StatusCode>,
}

impl LoggingInfo {
    fn new(method: Method, uri: Uri, ip: IpAddr) -> Self {
        Self {
            method,
            uri,
            ip,
            from_proxy: None,
            status_code: None,
        }
    }

    fn set_status_code(&mut self, status: StatusCode) {
        self.status_code = Some(status);
    }

    fn set_proxy_name(&mut self, proxy: AppOrProxyId) {
        self.from_proxy = Some(proxy);
    }

    fn get_log(&self) -> String {
        let from = self
            .from_proxy
            .as_ref()
            .map(|id| id.hide_broker())
            .unwrap_or(self.ip.to_string());
        format!(
            "{} {} {} {}",
            from,
            self.status_code
                .expect("Did not set Statuscode before logging"),
            self.method,
            self.uri
        )
    }
}

pub type ProxyLogger = mpsc::Sender<AppOrProxyId>;

pub async fn log(
    ConnectInfo(info): ConnectInfo<SocketAddr>,
    mut req: Request,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let ip = get_ip(&req, &info);

    let mut info = LoggingInfo::new(method, uri, ip);
    // This channel may or may not receive an AppOrProxyId from verify_with_extended_header
    // TODO: Solve this with tracing
    let (tx, mut rx) = mpsc::channel(1);
    req.extensions_mut().insert(tx);

    let resp = next.run(req).await;
    info.set_status_code(resp.status());

    if let Ok(proxy) = rx.try_recv() {
        info.set_proxy_name(proxy);
    }

    let line = info.get_log();
    // If we get a gateway timeout we won't log it with log level warn as this happens regularly with the long polling api
    if resp.status().is_success() || resp.status().is_informational() || resp.status() == StatusCode::GATEWAY_TIMEOUT {
        info!(target: "in", "{}", line);
    } else {
        warn!(target: "in", "{}", line);
    }
    resp
}

fn get_ip(req: &Request, info: &SocketAddr) -> IpAddr {
    req.headers()
        .get(X_FORWARDED_FOR)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(info.ip())
}
