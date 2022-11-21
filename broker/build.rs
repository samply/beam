use build_data::get_git_dirty;

/// Outputs a readable version number such as
/// 0.4.0 (if git commit is clean)
/// 0.4.0-SNAPSHOT (if git commit is dirty, should not happen in CI/CD builds)
fn version() -> String {
    let version = String::from(env!("CARGO_PKG_VERSION"));
    match get_git_dirty().unwrap() {
        false => {
            version
        },
        true => {
            format!("{}-SNAPSHOT", version)
        }
    }
}

fn main() {
    build_data::set_GIT_COMMIT_SHORT();
    build_data::set_GIT_DIRTY();
    build_data::set_BUILD_DATE();
    build_data::set_BUILD_TIME();
    build_data::no_debug_rebuilds();
    println!("cargo:rustc-env=SAMPLY_USER_AGENT=samply.beam.{}/{}", env!("CARGO_PKG_NAME"), version());
}
