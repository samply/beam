use axum::{
    middleware::Next,
    response::Response,
};
use http::{StatusCode, Request};
use hyper::Body;
use tracing::{info, warn, info_span, field, Instrument, Span};

use beam_lib::AppOrProxyId;

pub async fn log(
    req: Request<Body>,
    next: Next<Body>,
) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let span = info_span!("", from = field::Empty);

    async move {
        let resp = next.run(req).instrument(Span::current()).await;
        let status = resp.status();
        // If we get a gateway timeout we won't log it with log level warn as this happens regularly with the long polling api
        if resp.status().is_success() || resp.status().is_informational() || resp.status() == StatusCode::GATEWAY_TIMEOUT {
            info!(target: "in", "{method} {uri} {status}");
        } else {
            warn!(target: "in", "{method} {uri} {status}");
        };
        resp
    }.instrument(span).await
}
