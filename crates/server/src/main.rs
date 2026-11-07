//! MineRust server entry point.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};

/// Default configuration file, looked up in the current directory.
const DEFAULT_CONFIG: &str = "server.toml";

fn main() -> ExitCode {
    match try_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            // Tracing may not be initialised yet, so report startup failures
            // directly.
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
    let game_data = data::load_from_jar(&config.client_jar).with_context(|| {
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
