//! Server configuration: a TOML file loaded at startup.
//!
//! The file is intentionally **not** stored in the repository: it contains
//! the protocol version number, version name and client jar path of a
//! particular game release, which are taken from the user's own copy of the
//! game. See `server.example.toml` for the shape of the file.

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
    /// Path to the client jar of the user's game copy.
    ///
    /// Registries and tags are loaded from `data/minecraft/` inside it at
    /// startup; take the path from your own installation of the game.
    pub client_jar: String,
    /// Optional path to the data generator's `reports/packets.json`.
    ///
    /// When set, every packet id the server speaks is taken from that report
    /// (generated from the user's own copy of the game) and checked against
    /// the compiled defaults; without it the compiled defaults are used and
    /// the ids are unverified. See `tools/generate-reports.ps1`.
    #[serde(default)]
    pub packets_report: Option<String>,
    /// Optional path to the data generator's `reports/registries.json`.
    ///
    /// When set, the numeric ids of the built-in registries (`block`,
    /// `item`, `potion`, ...) come from that report, so the server can send
    /// their tags during configuration; without it only the synchronized
    /// registries get tags and the client fails to bind dimension types and
    /// enchantments. See `tools/generate-reports.ps1`.
    #[serde(default)]
    pub registries_report: Option<String>,
    /// Optional path to the official vanilla `server.jar` (M3 parity tests).
    ///
    /// Used only by the ignored parity tests of the `parity` crate, never by
    /// the running server. Missing means "parity tests unavailable".
    #[serde(default)]
    pub vanilla_server_jar: Option<String>,
    /// Optional working directory of the vanilla server installation.
    ///
    /// Contains `server.properties`, `logs/` and per-run world copies; used
    /// only by the ignored parity tests of the `parity` crate.
    #[serde(default)]
    pub vanilla_server_dir: Option<String>,
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
        if self.client_jar.trim().is_empty() {
            return Err(ConfigError::Invalid("client_jar is empty".to_owned()));
        }
        if self
            .packets_report
            .as_deref()
            .is_some_and(|path| path.trim().is_empty())
        {
            return Err(ConfigError::Invalid("packets_report is empty".to_owned()));
        }
        if self
            .registries_report
            .as_deref()
            .is_some_and(|path| path.trim().is_empty())
        {
            return Err(ConfigError::Invalid(
                "registries_report is empty".to_owned(),
            ));
        }
        if self
            .vanilla_server_jar
            .as_deref()
            .is_some_and(|path| path.trim().is_empty())
        {
            return Err(ConfigError::Invalid(
                "vanilla_server_jar is empty".to_owned(),
            ));
        }
        if self
            .vanilla_server_dir
            .as_deref()
            .is_some_and(|path| path.trim().is_empty())
        {
            return Err(ConfigError::Invalid(
                "vanilla_server_dir is empty".to_owned(),
            ));
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
client_jar = "client.jar"
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
        assert_eq!(config.client_jar, "client.jar");
        config.validate().expect("valid");
    }

    #[test]
    fn rejects_an_empty_client_jar_path() {
        let text = example_toml().replace("client.jar", "");
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn report_path_is_optional() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(config.packets_report, None);

        let text = example_toml()
            + r#"packets_report = "generated/reports/packets.json"
"#;
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(
            config.packets_report.as_deref(),
            Some("generated/reports/packets.json")
        );
        config.validate().expect("valid");
    }

    #[test]
    fn rejects_an_empty_report_path() {
        let text = example_toml()
            + r#"packets_report = " "
"#;
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn registries_report_path_is_optional() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(config.registries_report, None);

        let text = example_toml()
            + r#"registries_report = "generated/reports/registries.json"
"#;
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(
            config.registries_report.as_deref(),
            Some("generated/reports/registries.json")
        );
        config.validate().expect("valid");
    }

    #[test]
    fn rejects_an_empty_registries_report_path() {
        let text = example_toml()
            + r#"registries_report = " "
"#;
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn vanilla_paths_are_optional() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(config.vanilla_server_jar, None);
        assert_eq!(config.vanilla_server_dir, None);
        config.validate().expect("valid");
    }

    #[test]
    fn parses_vanilla_paths() {
        let text = example_toml()
            + r#"vanilla_server_jar = "C:/data/server.jar"
vanilla_server_dir = "C:/data/vanilla"
"#;
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(
            config.vanilla_server_jar.as_deref(),
            Some("C:/data/server.jar")
        );
        assert_eq!(
            config.vanilla_server_dir.as_deref(),
            Some("C:/data/vanilla")
        );
        config.validate().expect("valid");
    }

    #[test]
    fn rejects_empty_vanilla_paths() {
        for field in ["vanilla_server_jar", "vanilla_server_dir"] {
            let text = example_toml() + &format!("{field} = \" \"\n");
            let config: ServerConfig = toml::from_str(&text).expect("valid toml");
            assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
        }
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
