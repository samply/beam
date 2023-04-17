use http::HeaderValue;
use tracing::{debug, warn};

pub enum Verdict {
    BeamWithMatchingVersion,
    BeamWithMismatchingVersion(String),
    BeamWithInvalidVersion(String),
    NotBeam,
}

pub fn compare_version(their_version_header: &HeaderValue) -> Verdict {
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
        warn!(
            "Samply.Beam.Proxy gave invalid version number in User-Agent header: {}",
            their_version
        );
        return Verdict::BeamWithInvalidVersion(their_version.into());
    };

    if version_number != env!("CARGO_PKG_VERSION") {
        warn!(
            "Samply.Beam.Proxy has version mismatch: ours={}, theirs={}",
            env!("CARGO_PKG_VERSION"),
            version_number
        );
        return Verdict::BeamWithMismatchingVersion(version_number.into());
    };

    debug!(
        "Got health check with matching client version {}",
        version_number
    );

    Verdict::BeamWithMatchingVersion
}
