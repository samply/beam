use tracing::{Level, dispatcher::SetGlobalDefaultError};

pub fn init_logger() -> Result<(), SetGlobalDefaultError>{
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_env_filter("hyper=warn,rustify=warn,vaultrs=warn");

    #[cfg(debug_assertions)]
    let subscriber = subscriber.with_max_level(Level::DEBUG);

    let subscriber = subscriber.finish();
    tracing::subscriber::set_global_default(subscriber)?;
    
    tracing::debug!("Logging initialized.");
    Ok(())
}