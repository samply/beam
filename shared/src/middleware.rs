use std::net::{IpAddr, SocketAddr};

use axum::{middleware::{self, Next}, response::{Response, IntoResponse}, body::HttpBody, extract::ConnectInfo};
use http::{Request, StatusCode, header::{self, HeaderName}, HeaderValue};
use hyper::Body;
use tracing::{info, warn, span, Level, instrument};

const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

pub async fn log(ConnectInfo(info): ConnectInfo<SocketAddr>, req: Request<Body>, next: Next<Body>) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let ip = get_ip(&req, &info);

    let resp = next.run(req).await;

    let line = format!("{} {} {} {}",
        ip,
        resp.status().as_u16(),
        method,
        uri);
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