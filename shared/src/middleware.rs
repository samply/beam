use std::{net::{IpAddr, SocketAddr}, cell::RefCell, sync::Arc};

use axum::{middleware::{self, Next}, response::{Response, IntoResponse}, body::HttpBody, extract::ConnectInfo};
use http::{Request, StatusCode, header::{self, HeaderName}, HeaderValue, Method, Uri};
use hyper::Body;
use tokio::sync::{Mutex, oneshot};
use tracing::{info, warn, span, Level, instrument};

use crate::beam_id::AppOrProxyId;

const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

pub struct LoggingInfo {
    // Known from the start
    method: Method,
    uri: Uri,
    ip: IpAddr,

    // Added by SignedMsg extractor
    from_proxy: Option<AppOrProxyId>,

    // Added after handlers
    status_code: Option<StatusCode>
}

impl LoggingInfo {
    fn new(method: Method, uri: Uri, ip: IpAddr) -> Self {
        Self { method, uri, ip, from_proxy: None, status_code: None }
    }

    fn set_status_code(&mut self, status: StatusCode) {
        self.status_code = Some(status);
    }

    fn set_proxy_name(&mut self, proxy: AppOrProxyId) {
        self.from_proxy = Some(proxy);
    }

    fn get_log(&self) -> String {
        let from = self.from_proxy.as_ref().map(|id| id.to_string()).unwrap_or(self.ip.to_string());
        format!("{} {} {} {}",
            from,
            self.status_code
                .expect("Did not set Statuscode before loggin"),
            self.method,
            self.uri
        )
    }
}

pub type ProxyLogger = oneshot::Sender<AppOrProxyId>;

pub async fn log(ConnectInfo(info): ConnectInfo<SocketAddr>, mut req: Request<Body>, next: Next<Body>) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let ip = get_ip(&req, &info);

    let mut info = LoggingInfo::new(method, uri, ip);
    // This channel may or may not recieve an AppOrProxyId from verify_with_extended_header
    let (tx, mut rx) = oneshot::channel();
    req.extensions_mut().insert(tx);

    let resp = next.run(req).await;
    info.set_status_code(resp.status());

    if let Ok(proxy) = rx.try_recv() {
        info.set_proxy_name(proxy);
    }

    let line = info.get_log();
    if resp.status().is_success() {
        info!(target: "in", "{}", line);
    } else {
        warn!(target: "in", "{}", line);
    }
    resp
}

fn get_ip(req: &Request<Body>, info: &SocketAddr) -> IpAddr {
    req.headers().get(X_FORWARDED_FOR)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .and_then(|v| v.parse().ok())
        .unwrap_or(info.ip())
}