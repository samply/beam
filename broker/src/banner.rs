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
    if !response.headers_mut().contains_key(header::SERVER) {
        response.headers_mut().insert(
            header::SERVER,
            HeaderValue::from_static(env!("SAMPLY_USER_AGENT")),
        );
    }
    response
}
