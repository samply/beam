fn main() {
    build_data::set_GIT_COMMIT_SHORT();
    build_data::set_GIT_DIRTY();
    build_data::set_BUILD_DATE();
    build_data::set_BUILD_TIME();
    build_data::no_debug_rebuilds();
    println!("cargo:rustc-env=BROKER_AGENT=beam.broker/{}", env!("CARGO_PKG_VERSION"));
}
