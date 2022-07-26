use tracing::{dispatcher::SetGlobalDefaultError, Level, debug};

pub fn init_logger() -> Result<(), SetGlobalDefaultError>{
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(Level::DEBUG);

    let env_filter = match std::env::var("RUST_LOG") {
        Ok(env) if ! env.is_empty() => {
            env
        },
        _ => {
            if cfg!(debug_assertions) {
                "hyper=warn,rustify=warn,vaultrs=warn,info".to_string()
            } else {
                "hyper=warn,rustify=warn,vaultrs=warn,debug".to_string()
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
