use tracing::{debug, dispatcher::SetGlobalDefaultError, Level};
use tracing_subscriber::fmt::format::{debug_fn, self};

#[allow(clippy::if_same_then_else)] // The redundant if-else serves documentation purposes
pub fn init_logger() -> Result<(), SetGlobalDefaultError> {
    let subscriber = tracing_subscriber::FmtSubscriber::builder()
        .fmt_fields(debug_fn(|w, f, v| match f.name() {
            "from" | "message" => write!(w, "{v:?}"),
            _ => write!(w, "{f}={v:?} "),
        }))
        .with_max_level(Level::DEBUG);

    // TODO: Reduce code complexity.
    let env_filter = match std::env::var("RUST_LOG") {
        Ok(env) if !env.is_empty() => {
            if env.contains("hyper=") {
                env
            } else {
                format!("{env},hyper=info")
            }
        }
        _ => {
            if cfg!(debug_assertions) {
                "info,hyper=info".to_string()
            } else {
                "info,hyper=info".to_string()
            }
        }
    };

    let subscriber = subscriber.with_env_filter(env_filter.clone()).finish();
    tracing::subscriber::set_global_default(subscriber)?;

    debug!("Logging initialized with env_filter {env_filter}.");
    Ok(())
}
