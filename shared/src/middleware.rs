use std::net::IpAddr;

use axum::{middleware::{self, Next}, response::{Response, IntoResponse}, body::HttpBody};
use http::{Request, StatusCode, header::{self, HeaderName}, HeaderValue};
use hyper::Body;
use tracing::{info, warn, span, Level, instrument};

const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

pub async fn log(req: Request<Body>, next: Next<Body>) -> Response {
    let ip = get_ip(&req)
        .and_then(|f| Some(f.to_string()))
        .unwrap_or("NO_IP_HEADER".into());
    
    let mut line = format!("({}) <= {} {}", ip, req.method(), req.uri());
    let resp = next.run(req).await;
    #[allow(clippy::format_push_string)] // see https://github.com/rust-lang/rust-clippy/issues/9077
    line.push_str(&format!(" || => {}", resp.status()));
    if resp.status().is_success() {
        info!("{}", line);
    } else {
        warn!("{}", line);
    }
    resp
}

fn get_ip(req: &Request<Body>) -> Option<IpAddr> {
    if let Some(ip_from_header) = req.headers().get(X_FORWARDED_FOR) {
        let a = ip_from_header
            .to_str()
            .ok()?
            .split(',').next().unwrap_or_default()
            .parse()
            .ok()?;
        Some(a)
    } else {
        None
    }
}