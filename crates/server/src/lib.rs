//! Server library: configuration loading and the startup sequence.
//!
//! The binary in `main.rs` is a thin wrapper around [`run`]. Keeping the
//! logic in a library allows the integration tests to start a real server on
//! an ephemeral port without spawning a subprocess.

mod config;

pub use config::{load_config, ConfigError, ServerConfig};

use std::sync::Arc;

use protocol::StatusInfo;
use tokio::net::TcpListener;

/// Builds the status document advertised to the multiplayer server list.
///
/// The player count is hard-coded to 0 for now: tracking online players
/// belongs to milestone M2 (login).
pub fn status_info(config: &ServerConfig) -> StatusInfo {
    StatusInfo {
        version_name: config.version_name.clone(),
        protocol_version: config.protocol_version,
        max_players: config.max_players,
        online_players: 0,
        motd: config.motd.clone(),
    }
}

/// Binds the configured address and serves connections until an I/O error.
pub async fn run(config: ServerConfig) -> std::io::Result<()> {
    let listener = TcpListener::bind(&config.listen).await?;
    let local = listener.local_addr()?;
    tracing::info!(
        %local,
        motd = %config.motd,
        max_players = config.max_players,
        protocol_version = config.protocol_version,
        version_name = %config.version_name,
        "listening"
    );
    net::serve(listener, Arc::new(status_info(&config))).await
}
