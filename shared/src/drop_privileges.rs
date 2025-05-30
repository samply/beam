use privdrop::PrivDrop;
use tracing::{error, warn};

use crate::config;

pub fn drop_privileges_or_fail() {
    if let Err(err) = PrivDrop::default()
        .user("nobody")
        .group("nobody")
        .apply() {
            if config::CONFIG_SHARED.require_nonroot {
                error!("Failed to drop privileges: {err}");
                std::process::exit(1);
            } else {
                warn!("Failed to drop privileges: {err}; continuing as root.")
            }
        }
}