use tracing::{Level, dispatcher::SetGlobalDefaultError};

pub fn init_logger() -> Result<(), SetGlobalDefaultError>{
    let subscriber = tracing_subscriber::FmtSubscriber::builder();

    #[cfg(debug_assertions)]
    let subscriber = subscriber.with_max_level(Level::DEBUG);

    let subscriber = subscriber.finish();
    tracing::subscriber::set_global_default(subscriber)?;
    
    tracing::debug!("Logging initialized.");
    Ok(())
}