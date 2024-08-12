use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::Response,
};
use tracing::{info, warn, info_span, field, Instrument, Span};

pub async fn log(
    req: Request,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let span = info_span!("", from = field::Empty);

    async move {
        let resp = next.run(req).instrument(Span::current()).await;
        let status = resp.status();
        // If we get a gateway timeout we won't log it with log level warn as this happens regularly with the long polling api
        if status.is_success() || status.is_informational() || status == StatusCode::GATEWAY_TIMEOUT {
            info!(target: "in", "{method} {uri} {status}");
        } else {
            warn!(target: "in", "{method} {uri} {status}");
        };
        resp
    }.instrument(span).await
}
