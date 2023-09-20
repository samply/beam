use axum::{middleware::Next, response::Response};
use hyper::{header, Request, Body, http::HeaderValue};
use tracing::{debug, warn};

enum Verdict {
    BeamWithMatchingVersion,
    BeamWithMismatchingVersion(String),
    BeamWithInvalidVersion(String),
    NotBeam,
}

fn compare_version(their_version_header: &HeaderValue) -> Verdict {
    let mut version = their_version_header
        .to_str()
        .unwrap_or("GARBLED_CLIENT_VERSION")
        .split('/');

    let Some(val) = version.next() else {
        return Verdict::NotBeam;
    };

    if val.to_lowercase() != "samply.beam.proxy" {
        return Verdict::NotBeam;
    }

    let Some(version_number) = version.next() else {
        let their_version = their_version_header.to_str().unwrap_or("GARBLED");
        return Verdict::BeamWithInvalidVersion(their_version.into());
    };
    // Remove git hash from builds if present as we only care about a semver mismatch
    let version_number = version_number.split_once('-').map(|v| v.0).unwrap_or(version_number);

    if version_number != env!("CARGO_PKG_VERSION") {
        return Verdict::BeamWithMismatchingVersion(version_number.into());
    };
    Verdict::BeamWithMatchingVersion
}

pub(crate) async fn log_version_mismatch(
    req: Request<Body>,
    next: Next<Body>,
) -> Response {
    let user_agent = req.headers().get(header::USER_AGENT);

    if let Some(their_version_header) = user_agent {
        match compare_version(&their_version_header) {
            Verdict::BeamWithMatchingVersion | Verdict::NotBeam => (),
            Verdict::BeamWithMismatchingVersion(v) => warn!("Samply.Beam.Proxy has version mismatch: ours={}, theirs={v}", env!("CARGO_PKG_VERSION")),
            Verdict::BeamWithInvalidVersion(v) => warn!("Samply.Beam.Proxy gave invalid version number in User-Agent header: {v}"),
        }
    } else {
        warn!("No User-Agent supplied");
    }
    next.run(req).await
}
