use tracing::{dispatcher::SetGlobalDefaultError, Level, debug};

#[allow(clippy::if_same_then_else)] // The redundant if-else serves documentation purposes
pub fn init_logger() -> Result<(), SetGlobalDefaultError>{
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(Level::DEBUG);

    let env_filter = match std::env::var("RUST_LOG") {
        Ok(env) if ! env.is_empty() => {
            env
        },
        _ => {
            if cfg!(debug_assertions) {
                "info,hyper=warn".to_string()
            } else {
                "info,hyper=warn".to_string()
            }
        }
    };

    let subscriber = subscriber
        .with_env_filter(env_filter.clone())
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;
    
    debug!("Logging initialized with env_filter {env_filter}.");
    Ok(())
}
