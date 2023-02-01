use axum::{middleware::{self, Next}, response::{Response, IntoResponse}, body::HttpBody};
use http::{Request, StatusCode, header};
use hyper::Body;
use tracing::{info, warn, span, Level, instrument};

#[instrument(skip_all,fields(req = make_line(&req)))]
pub async fn log(req: Request<Body>, next: Next<Body>) -> Response {
    // let log = format!("<= {} {}", req.method(), req.uri());
    // span!(Level::INFO, log);
    let mut line = format!("<= {} {}", req.method(), req.uri());
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

fn make_line(req: &Request<Body>) -> String {
    format!("{} {}", req.method(), req.uri())
}