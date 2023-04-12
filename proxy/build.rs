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
    println!("cargo:rustc-env=SAMPLY_USER_AGENT=Samply.Beam.{}/{}", env!("CARGO_PKG_NAME"), version());

    // Compile the frontend with npm but not when we are using cross in the CI because we can compile it ahead of time and dont need to install npm in the docker containers that cross will spawn
    if cfg!(feature = "monitor") && option_env!("CROSS_RUNNER").is_none() {
        use std::process::Command;
        std::env::set_current_dir("./monitor").expect("monitor directory has been deleted");
        Command::new("npm")
            .arg("install")
            .spawn()
            .expect("npm needs to be installed to compile monitoring feature")
            .wait()
            .expect("Failed to run npm install command");
        Command::new("npm")
            .args([ "run", "build" ])
            .spawn()
            .expect("npm needs to be installed to compile monitoring feature")
            .wait()
            .expect("Failed to run npm run build command");
    }
}
