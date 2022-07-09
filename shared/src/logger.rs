use tracing::{Level, dispatcher::SetGlobalDefaultError, debug, info};

pub fn init_logger() -> Result<(), SetGlobalDefaultError>{
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(Level::INFO);

    let subscriber = subscriber
        .with_env_filter("hyper=warn,rustify=warn,vaultrs=warn,info");

    // #[cfg(debug_assertions)]
    // let subscriber = subscriber.with_max_level(Level::DEBUG);

    let subscriber = subscriber.finish();
    tracing::subscriber::set_global_default(subscriber)?;
    
    debug!("Logging initialized.");
    Ok(())
}
