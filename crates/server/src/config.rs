//! Server configuration: a TOML file loaded at startup.
//!
//! The file is intentionally **not** stored in the repository: it contains the
//! protocol version number and version name of a particular game release,
//! which are taken from the user's own copy of the game. See
//! `server.example.toml` for the shape of the file.

use std::path::Path;

use serde::Deserialize;

/// Errors while loading or validating the server configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The configuration file could not be read.
    #[error("failed to read config {path}: {source}")]
    Io {
        /// Path that could not be read.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The configuration file is not valid TOML or has the wrong shape.
    #[error("failed to parse config {path}: {source}")]
    Parse {
        /// Path that could not be parsed.
        path: String,
        /// Underlying TOML error.
        #[source]
        source: toml::de::Error,
    },
    /// A field failed validation.
    #[error("invalid config: {0}")]
    Invalid(String),
}

/// Contents of `server.toml`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// Address to listen on, e.g. `0.0.0.0:25565` or `127.0.0.1:25565`.
    pub listen: String,
    /// Message of the day shown in the server list.
    pub motd: String,
    /// Maximum number of players.
    pub max_players: i32,
    /// Protocol version reported to clients.
    ///
    /// Must match the client's protocol version for the client to treat the
    /// server as compatible; take it from your own copy of the game.
    pub protocol_version: i32,
    /// Version name displayed in the server list.
    pub version_name: String,
}

impl ServerConfig {
    /// Validates fields that must be non-empty or positive.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.listen.trim().is_empty() {
            return Err(ConfigError::Invalid("listen address is empty".to_owned()));
        }
        if self.max_players <= 0 {
            return Err(ConfigError::Invalid(format!(
                "max_players must be positive, got {}",
                self.max_players
            )));
        }
        if self.version_name.trim().is_empty() {
            return Err(ConfigError::Invalid("version_name is empty".to_owned()));
        }
        Ok(())
    }
}

/// Loads and validates a configuration file from `path`.
pub fn load_config(path: impl AsRef<Path>) -> Result<ServerConfig, ConfigError> {
    let path_ref = path.as_ref();
    let display = path_ref.display().to_string();
    let text = std::fs::read_to_string(path_ref).map_err(|source| ConfigError::Io {
        path: display.clone(),
        source,
    })?;
    let config: ServerConfig = toml::from_str(&text).map_err(|source| ConfigError::Parse {
        path: display,
        source,
    })?;
    config.validate()?;
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_toml() -> String {
        r#"
listen = "127.0.0.1:25565"
motd = "A MineRust server"
max_players = 20
protocol_version = 4242
version_name = "test-version"
"#
        .to_owned()
    }

    #[test]
    fn parses_a_valid_config() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(config.listen, "127.0.0.1:25565");
        assert_eq!(config.max_players, 20);
        assert_eq!(config.protocol_version, 4242);
        assert_eq!(config.version_name, "test-version");
        config.validate().expect("valid");
    }

    #[test]
    fn rejects_unknown_fields() {
        let mut text = example_toml();
        text.push_str("surprise = true\n");
        let err = toml::from_str::<ServerConfig>(&text).expect_err("unknown field");
        assert!(err.to_string().contains("surprise"));
    }

    #[test]
    fn rejects_non_positive_max_players() {
        let text = example_toml().replace("max_players = 20", "max_players = 0");
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn rejects_empty_listen_address() {
        let text = example_toml().replace("127.0.0.1:25565", "");
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert!(config.validate().is_err());
    }

    #[test]
    fn reports_a_missing_file() {
        let err = load_config("definitely-not-here.toml").expect_err("missing");
        assert!(matches!(err, ConfigError::Io { .. }));
    }
}
