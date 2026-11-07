//! Server library: configuration loading and the startup sequence.
//!
//! The binary in `main.rs` is a thin wrapper around [`run`]. Keeping the
//! logic in a library allows the integration tests to start a real server on
//! an ephemeral port without spawning a subprocess.

mod config;

pub use config::{load_config, ConfigError, ServerConfig};

use std::sync::Arc;

use data::GameData;
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

/// Errors of the startup sequence: config-time I/O and game data problems.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The listener could not be bound or a connection layer I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Game data (jar, reports) could not be read or did not verify.
    #[error(transparent)]
    Data(#[from] data::DataError),
}

/// Loads the packet ids the server will speak.
///
/// With `config.packets_report` set the ids come from the data generator's
/// report of the user's own game copy and every packet key must be covered;
/// without it the compiled defaults are used (SOURCE: wiki, unverified) and
/// the caller is told so by a warning.
fn load_packet_ids(config: &ServerConfig) -> Result<Arc<data::PacketIds>, RunError> {
    let Some(path) = config.packets_report.as_deref() else {
        tracing::warn!(
            "no packets_report configured, using packet ids from the public \
             protocol documentation (unverified); run tools/generate-reports.ps1"
        );
        return Ok(Arc::new(data::PacketIds::compiled()));
    };
    let ids = data::load_packets_report(path)?;
    ids.verify(protocol::used_packet_keys())?;
    tracing::info!(
        path,
        ids = ids.len(),
        "packet ids loaded from the packets report"
    );
    Ok(Arc::new(ids))
}

/// Binds the configured address and serves connections until an I/O error.
///
/// `game_data` holds the registries and tags loaded from the client jar
/// (see `data::load_from_jar`); the caller loads it once at startup so a
/// broken jar fails before the port is bound. Packet ids are resolved the
/// same way: a broken packets report fails before the port is bound.
pub async fn run(config: ServerConfig, game_data: Arc<GameData>) -> Result<(), RunError> {
    let packet_ids = load_packet_ids(&config)?;
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
    net::serve(
        listener,
        Arc::new(status_info(&config)),
        game_data,
        packet_ids,
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::StatusResponse;

    /// Test config: the fields the id loader does not read may be anything.
    fn config(report: Option<String>) -> ServerConfig {
        ServerConfig {
            listen: "127.0.0.1:0".to_owned(),
            motd: "test".to_owned(),
            max_players: 1,
            protocol_version: 1,
            version_name: "test".to_owned(),
            client_jar: "client.jar".to_owned(),
            packets_report: report,
            registries_report: None,
            vanilla_server_jar: None,
            vanilla_server_dir: None,
        }
    }

    #[test]
    fn without_a_report_the_compiled_ids_are_used() {
        let ids = load_packet_ids(&config(None)).expect("compiled ids never fail");
        assert!(ids.is_empty());
        assert_eq!(ids.id(StatusResponse::KEY), StatusResponse::ID);
    }

    #[test]
    fn a_report_provides_the_ids() {
        // Fictional override: proof that the id comes from the report.
        let json = data::fixtures::packets_report_json(&[(StatusResponse::KEY, 0x7F)]);
        let path = std::env::temp_dir().join("minerust-packets-report-test.json");
        std::fs::write(&path, &json).expect("write fixture report");
        let ids = load_packet_ids(&config(Some(path.display().to_string()))).expect("valid report");
        assert_eq!(ids.id(StatusResponse::KEY), 0x7F);
    }

    #[test]
    fn a_broken_report_is_an_error_before_binding() {
        let error = load_packet_ids(&config(Some("definitely-not-here.json".to_owned())))
            .expect_err("missing report");
        assert!(matches!(error, RunError::Data(_)));
    }

    #[test]
    fn a_report_with_a_missing_packet_is_rejected() {
        // `minecraft:login` sorts into the middle of the play/clientbound
        // entries, so the entry with its leading comma can be dropped and
        // the remaining report stays valid JSON.
        let key = protocol::LoginPlay::KEY;
        let json = data::fixtures::packets_report_json(&[]);
        let text = json.replace(
            &format!(",\"{}\":{{\"protocol_id\":{}}}", key.name, key.default_id),
            "",
        );
        assert_ne!(text.len(), json.len(), "the key must have been removed");
        let path = std::env::temp_dir().join("minerust-packets-report-incomplete.json");
        std::fs::write(&path, &text).expect("write fixture report");
        let error = load_packet_ids(&config(Some(path.display().to_string())))
            .expect_err("incomplete report");
        assert!(matches!(error, RunError::Data(_)), "{error}");
    }
}
