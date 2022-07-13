use tracing::info;

pub fn print_banner() {
    let commit = match env!("GIT_DIRTY") {
        "false" => {
            env!("GIT_COMMIT_SHORT")
        },
        _ => {
            "SNAPSHOT"
        }
    };
    info!("Samply.Beam ({}) v{} (built {} {}, {}) starting up ...", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"), env!("BUILD_DATE"), env!("BUILD_TIME"), commit);
}