//! Server configuration: a TOML file loaded at startup.
//!
//! The file is intentionally **not** stored in the repository: it contains
//! the protocol version number, version name and client jar path of a
//! particular game release, which are taken from the user's own copy of the
//! game. See `server.example.toml` for the shape of the file.

use std::path::Path;

use serde::Deserialize;

/// Value both chunk distances fall back to when the field is absent: the
/// vanilla defaults the parity test `login_fields` reads out of the real
/// server's join (`view_distance = 10`, `simulation_distance = 10`, the
/// same numbers its `server.properties` slider advertises).
pub const DEFAULT_DISTANCES: i32 = 10;

/// Serde default shared by `view_distance` and `simulation_distance`.
fn default_distances() -> i32 {
    DEFAULT_DISTANCES
}

/// Serde default of `workers` (M4.3 A2): logical cores − 2, at least 1.
///
/// The two spare cores are left to the network/tokio threads and the OS, so
/// the region workers do not starve the connections. Set `workers = 0` to
/// disable the pool and tick every region on the game thread (the pre-M4.3
/// behaviour).
fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map(|cores| cores.get().saturating_sub(2).max(1))
        .unwrap_or(1)
}

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
    /// Optional path to the data generator's `reports/blocks.json`.
    ///
    /// When set, the numeric state ids of every block (`default.properties`
    /// flattened per state) come from that report, which is what turns the
    /// block palettes stored in region files into the ids the network
    /// palette expects; without it chunk sections cannot be encoded. See
    /// `tools/generate-reports.ps1`.
    #[serde(default)]
    pub blocks_report: Option<String>,
    /// Chunks around the player the server announces it sends in Login
    /// (play) — also the largest radius this server ever sends (the real
    /// radius is min(this, the client's own request), see
    /// `net::connection::play::send_radius`).
    ///
    /// Absent means [`DEFAULT_DISTANCES`] (both vanilla login fields
    /// recorded by the parity test `login_fields` are 10).
    #[serde(default = "default_distances")]
    pub view_distance: i32,
    /// Chunks around the player the server announces it keeps simulating
    /// in Login (play). Nothing is simulated yet — this only announces the
    /// number (M4 and later do the work); absent means
    /// [`DEFAULT_DISTANCES`] like `view_distance`.
    #[serde(default = "default_distances")]
    pub simulation_distance: i32,
    /// Worker threads of the region pool (M4.3): how many regions are ticked
    /// in parallel. `0` disables the pool and ticks every region on the game
    /// thread (the M4.2 behaviour). Absent means [`default_workers`]:
    /// logical cores − 2, at least 1.
    #[serde(default = "default_workers")]
    pub workers: usize,
    /// Side of a region tile in chunks (M4.3 part B). `0` — the default —
    /// means "no split": the whole world is one region, exactly the pre-M4.3
    /// behaviour, so nothing changes on the live server until the owner
    /// raises it. The value is unverified by a benchmark yet (see D3 and
    /// `docs/DECISIONS.md`): M4.6 decides the side.
    #[serde(default)]
    pub region_side: i32,
    /// Optional path to a saved world directory (step 3.3, M3).
    ///
    /// The directory is a world save: it holds `level.dat` (whose
    /// `Data.DataVersion` is the schema every sent chunk must match) and
    /// `dimensions/minecraft/overworld/region/` with the region files of
    /// the overworld. When set, the server sends the ready chunks of that
    /// world on join; when omitted, no chunks are sent and the join state
    /// is the empty world of M2.
    #[serde(default)]
    pub world_dir: Option<String>,
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
        if self.view_distance <= 0 {
            return Err(ConfigError::Invalid(format!(
                "view_distance must be positive (chunks), got {}",
                self.view_distance
            )));
        }
        if self.simulation_distance <= 0 {
            return Err(ConfigError::Invalid(format!(
                "simulation_distance must be positive (chunks), got {}",
                self.simulation_distance
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
            .blocks_report
            .as_deref()
            .is_some_and(|path| path.trim().is_empty())
        {
            return Err(ConfigError::Invalid("blocks_report is empty".to_owned()));
        }
        if self
            .world_dir
            .as_deref()
            .is_some_and(|path| path.trim().is_empty())
        {
            return Err(ConfigError::Invalid("world_dir is empty".to_owned()));
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
    fn both_chunk_distances_default_to_the_vanilla_ten() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(config.view_distance, DEFAULT_DISTANCES, "default view");
        assert_eq!(
            config.simulation_distance, DEFAULT_DISTANCES,
            "default simulation"
        );
        config.validate().expect("valid");
    }

    #[test]
    fn parses_chunk_distances_from_the_file() {
        let text = example_toml()
            + r#"view_distance = 6
simulation_distance = 4
"#;
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(config.view_distance, 6);
        assert_eq!(config.simulation_distance, 4);
        config.validate().expect("valid");
    }

    #[test]
    fn workers_default_to_the_cores_minus_two_and_zero_disables_the_pool() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        let expected = default_workers();
        let cores = std::thread::available_parallelism()
            .map(|cores| cores.get())
            .unwrap_or(1);
        assert_eq!(config.workers, expected, "absent means the serde default");
        assert_eq!(expected, cores.saturating_sub(2).max(1), "cores - 2, min 1");

        let text = example_toml() + "workers = 0\n";
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(config.workers, 0, "0 disables the pool");
        config.validate().expect("valid");

        let text = example_toml() + "workers = 4\n";
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(config.workers, 4);
    }

    #[test]
    fn region_side_defaults_to_zero_and_parses_an_override() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(config.region_side, 0, "absent means no split");

        let text = example_toml() + "region_side = 64\n";
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(config.region_side, 64);
        config.validate().expect("valid");
    }

    #[test]
    fn rejects_a_distance_that_is_not_positive() {
        for field in ["view_distance", "simulation_distance"] {
            let text = example_toml() + &format!("{field} = 0\n");
            let config: ServerConfig = toml::from_str(&text).expect("valid toml");
            assert!(
                matches!(config.validate(), Err(ConfigError::Invalid(_))),
                "{field} = 0 must be rejected"
            );
        }
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
    fn blocks_report_path_is_optional() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(config.blocks_report, None);

        let text = example_toml()
            + r#"blocks_report = "generated/reports/blocks.json"
"#;
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(
            config.blocks_report.as_deref(),
            Some("generated/reports/blocks.json")
        );
        config.validate().expect("valid");
    }

    #[test]
    fn rejects_an_empty_blocks_report_path() {
        let text = example_toml()
            + r#"blocks_report = " "
"#;
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn world_dir_is_optional_and_validated() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(config.world_dir, None, "no world means no chunks");
        config.validate().expect("valid");

        let text = example_toml() + "world_dir = \"C:/data/world-template\"\n";
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(config.world_dir.as_deref(), Some("C:/data/world-template"));
        config.validate().expect("valid");

        let empty = example_toml() + "world_dir = \" \"\n";
        let config: ServerConfig = toml::from_str(&empty).expect("valid toml");
        assert!(matches!(
            config.validate(),
            Err(ConfigError::Invalid(_)) if config.world_dir.is_some()
        ));
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
