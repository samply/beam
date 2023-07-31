/// Outputs a readable version number such as
/// 0.4.0 (if git commit is on branch main)
/// 0.4.0-a12dds (if git commit is clean and on another branch than main)
/// 0.4.0-a12dds-SNAPSHOT (if git commit is dirty, should not happen in CI/CD builds)
fn get_version() -> String {
    let mut version = String::from(env!("CARGO_PKG_VERSION"));
    let (branch, commit) = match (build_data::get_git_branch(), build_data::get_git_commit_short()) {
        (Ok(branch), Ok(commit)) => (branch, commit),
        _ => {
            println!("cargo:warning=Unable to read git info. Is this a git repository?");
            return version;
        }
    };
    if branch != "main" {
        version.push_str(&format!("-{commit}"));
        if build_data::get_git_dirty().unwrap_or(true) {
            version = format!("{}-SNAPSHOT", version)
        }
    };
    version
}

fn get_pkg_name() -> String {
    let pkg_name_raw = env!("CARGO_PKG_NAME");
    let mut pkg_name = String::with_capacity(pkg_name_raw.len());
    let mut first = true;
    for char in pkg_name_raw.chars() {
        if first {
            first = false;
            pkg_name.push(char.to_ascii_uppercase())
        } else {
            pkg_name.push(char.to_ascii_lowercase())
        }
    }
    pkg_name
}

fn set_samply_user_agent() {
    println!(
        "cargo:rustc-env=SAMPLY_USER_AGENT=Samply.Beam.{}/{}",
        get_pkg_name(),
        get_version()
    );
}

fn set_features() {
    let env_vars: Vec<_> = std::env::vars().collect();
    println!(
        "cargo:rustc-env=FEATURES={}",
            env_vars.iter()
                .filter_map(|(name, _)| name.strip_prefix("CARGO_FEATURE_"))
                .collect::<Vec<_>>()
                .join(", ")
                .to_lowercase()
    );
}

fn main() {
    build_data::set_GIT_COMMIT_SHORT();
    build_data::set_GIT_DIRTY();
    build_data::set_BUILD_DATE();
    build_data::set_BUILD_TIME();
    set_features();
    set_samply_user_agent();
    build_data::no_debug_rebuilds();
}
