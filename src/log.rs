use anyhow::Result;
use tracing_appender::{non_blocking::WorkerGuard, rolling};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

use crate::paths;

pub fn init(verbose: bool) -> Result<WorkerGuard> {
    paths::ensure_dirs()?;
    let logs_dir = paths::logs_dir()?;
    let appender = rolling::daily(&logs_dir, "bisl-torah.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(appender);

    let default_level = if verbose { "debug" } else { "warn" };
    let env_filter = EnvFilter::try_from_env("BISL_TORAH_LOG")
        .or_else(|_| EnvFilter::try_new(default_level))
        .unwrap_or_else(|_| EnvFilter::new("warn"));

    let file_layer = fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(false);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .try_init()
        .ok(); // ignore if already set (e.g. tests)

    Ok(guard)
}
