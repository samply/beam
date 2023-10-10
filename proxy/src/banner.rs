use axum::{http::HeaderValue, response::Response};
use hyper::header;
use tracing::info;

pub(crate) fn print_banner() {
    let commit = match env!("GIT_DIRTY") {
        "false" => {
            env!("GIT_COMMIT_SHORT")
        }
        _ => "SNAPSHOT",
    };
    info!(
        "🌈 Samply.Beam ({}) v{} (built {} {}, {} with feature(s): {}) starting up ...",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
        env!("BUILD_DATE"),
        env!("BUILD_TIME"),
        commit,
        env!("FEATURES")
    );
}

pub(crate) async fn set_server_header<B>(mut response: Response<B>) -> Response<B> {
    let headers = response.headers_mut();
    let target_header = match headers.contains_key(header::SERVER) {
        true => header::VIA,     // We're relaying for a broker
        false => header::SERVER, // We're answering ourselves
    };
    headers.insert(
        target_header,
        HeaderValue::from_static(env!("SAMPLY_USER_AGENT")),
    );
    response
}
