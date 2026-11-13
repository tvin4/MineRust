//! MineRust server entry point.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};

/// Default configuration file, looked up in the current directory.
const DEFAULT_CONFIG: &str = "server.toml";

fn main() -> ExitCode {
    // Logs go through a non-blocking writer (M4.5 step 8): a line logged
    // from the tick thread only enters a bounded queue, and a separate writer
    // thread owns stdout, so a slow console cannot block a tick. `lossy(false)`
    // decides what a full queue does: the logging thread blocks until the
    // writer catches up (backpressure, `send` on a bounded channel) instead
    // of dropping lines - a pause reproduction needs every tick line. The
    // queue holds 128 000 lines by default (`DEFAULT_BUFFERED_LINES_LIMIT`),
    // so a healthy run never fills it. The guard below is the crate's drop
    // guard: on drop it signals the writer thread, waits up to one second for
    // it to flush the queue and joins it, so the last lines survive both a
    // clean exit and an error return.
    let (writer, _guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .lossy(false)
        .finish(std::io::stdout());
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    match try_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Tracing is initialised above and flushed by the guard; the
            // direct stderr copy gives the launcher a line to capture even
            // when the log filter hides errors.
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn try_main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let config_path = parse_args()?;
    let config = server::load_config(&config_path)
        .with_context(|| format!("failed to load config {}", config_path.display()))?;
    tracing::info!(path = %config_path.display(), "config loaded");

    // Load registries and tags from the user's client jar before binding,
    // so a broken jar path fails fast with a clear error.
    let game_data = data::load_from_jar(
        &config.client_jar,
        config.registries_report.as_deref().map(Path::new),
    )
    .with_context(|| {
        format!(
            "failed to load game data from client jar {}",
            config.client_jar
        )
    })?;
    let game_data = std::sync::Arc::new(game_data);

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;
    runtime.block_on(server::run(config, game_data))?;
    Ok(())
}

/// Parses command line arguments: only `--config <path>` is supported.
fn parse_args() -> Result<PathBuf> {
    let mut args = std::env::args().skip(1);
    let mut config_path = PathBuf::from(DEFAULT_CONFIG);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                let value = args.next().context("--config requires a path argument")?;
                config_path = PathBuf::from(value);
            }
            "--help" | "-h" => {
                println!("usage: server [--config <path>]");
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    Ok(config_path)
}
