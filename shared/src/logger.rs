use std::{io, path::PathBuf};

use tracing::{debug, dispatcher::SetGlobalDefaultError, Level};
use tracing_appender::{non_blocking::WorkerGuard, rolling::{InitError, Rotation}};
use tracing_subscriber::{fmt::{self, format::{self, debug_fn}}, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

#[derive(Debug, clap::Args)]
pub struct LogOptions {
    #[clap(long, env)]
    /// Directory to store log files in.
    pub log_dir: Option<PathBuf>,

    /// Number of days to retain log files.
    #[clap(long, env, default_value_t = 30)]
    pub log_retention: usize,
}

pub fn init_logger(log_opts: &LogOptions) -> Result<Option<WorkerGuard>, InitError> {
    let env_filter = EnvFilter::builder()
        .with_default_directive(Level::INFO.into())
        .from_env_lossy();
    let registry = tracing_subscriber::registry()
        .with(env_filter);
    let (registry, guard) = if let LogOptions { log_dir: Some(log_dir), log_retention } = log_opts {
        let appender = tracing_appender::rolling::Builder::new()
            .max_log_files(*log_retention)
            .filename_suffix("log")
            .rotation(Rotation::DAILY)
            .build(log_dir)?;
        let (appender, guard) = tracing_appender::non_blocking(appender);
        let json_layer = fmt::layer()
            .json()
            .with_writer(appender);
        (registry.with(Some(json_layer)), Some(guard))
    } else {
        (registry.with(None), None)
    };

    let stdout_layer = fmt::layer()
        .fmt_fields(debug_fn(|w, f, v| match f.name() {
            "from" | "message" => write!(w, "{v:?}"),
            _ => write!(w, "{f}={v:?} "),
        }));

    registry
        .with(stdout_layer)
        .init();

    Ok(guard)
}
