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

/// Serde default of `force_gamemode` (Шаг 18a): the configured mode wins.
fn default_true() -> bool {
    true
}

/// Serde default of `autosave_interval_ticks` (Шаг 18a): five minutes at
/// 20 TPS.
fn default_autosave_ticks() -> u64 {
    DEFAULT_AUTOSAVE_TICKS
}

/// Autosave period both the serde default and the example use.
pub const DEFAULT_AUTOSAVE_TICKS: u64 = 6000;

/// Serde default of `world_autosave_ticks` (Шаг 22b): the same five minutes
/// at 20 TPS as the player save, but an independent setting.
fn default_world_autosave_ticks() -> u64 {
    DEFAULT_AUTOSAVE_TICKS
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
    /// Optional path to a collision-shape report (`reports/collision.json`).
    ///
    /// It maps every non-empty block state id to the lowest Y of its collision
    /// boxes inside its cell. `reports/blocks.json` carries state ids but no
    /// shapes, so this report is what lets the server decide whether a player
    /// still fits standing under the block above it (Шаг 15 п.5, forced
    /// crouch/crawl). Generated from the user's own game copy by the test
    /// client's `shapes` command (`tools/testclient`); without it the forced
    /// pose stays off.
    #[serde(default)]
    pub collision_report: Option<String>,
    /// Optional path to a hardness report (`reports/hardness.json`).
    ///
    /// It maps every block state id to its hardness (destroy speed) and the
    /// bare-hand dig speed. `reports/blocks.json` carries no hardness, so this
    /// report is what the server-side mining timer (Шаг 16a) reads to estimate
    /// how long a dig lasts. Generated from the user's own game copy by the test
    /// client's `hardness` command (`tools/testclient`); without it the server
    /// keeps trusting the client about when a block is broken.
    #[serde(default)]
    pub hardness_report: Option<String>,
    /// Optional finish-time tolerance of the mining timer, in ticks (Шаг 16a.3).
    ///
    /// Absent (the default) trusts the client about when a block is finished:
    /// the crack stages still go out, but a `player_action` finish is never
    /// rolled back. Set it to refuse a steady survival finish that is more
    /// than that many ticks before the modelled `1.0`.
    ///
    /// Off by default since Шаг 18.0.1 (owner's decision): the 17b stand
    /// showed the vanilla 26.3 server accepting a finish one client tick into
    /// blocks as hard as `ender_chest`, so the old `Some(3)` (the 16a.4
    /// oracle's choice) was stricter than vanilla and rolled back an honest
    /// dig. The evidence is one stand, so the choice is temporary; see
    /// `docs/BLOCK-BREAKING.md` §1.11 and §6 п.9.
    #[serde(default)]
    pub dig_tolerance_ticks: Option<u32>,
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
    /// Raise the tick thread's priority one step above normal (A3/D7).
    ///
    /// Windows only: `SetThreadPriority` with `THREAD_PRIORITY_ABOVE_NORMAL`;
    /// other platforms ignore it. The A3 A/B series measured it under CPU
    /// contention (100 bots + 8 busy threads): the late-tick count dropped
    /// from 27/4/10 to 1/0/0 while the tick's own p50/p99 did not change — the
    /// flag only helps the OS schedule the tick's wake-ups, which is what the
    /// live `tick woke late` batches need (see `docs/DECISIONS.md` D7). Absent
    /// means `false` (the benchmarked default); turn it on when the server
    /// shares its machine with other heavy processes.
    #[serde(default)]
    pub tick_thread_priority: bool,
    /// Game mode every new player joins in (Шаг 14c): `0` survival (the
    /// default, the pre-14c behaviour), `1` creative, `2` adventure,
    /// `3` spectator. Absent means survival. There is no in-game mode switch
    /// yet, so the scenario runner raises it per run to place a player in
    /// creative.
    #[serde(default)]
    pub game_mode: i32,
    /// Optional development-only player spawn as `x,y,z` block coordinates.
    /// Does not change the world's default spawn or save files.
    #[serde(default)]
    pub spawn_override: Option<String>,
    /// Optional development-only geometry command file (Шаг 15 п.2).
    ///
    /// Off by default. When set, the server warns at startup and applies the
    /// `/setblock` and `/fill` commands of the file to the in-memory chunk
    /// store before the port is bound; the world directory is never written.
    /// The file holds one command per line, blank lines and `#` comments
    /// allowed (the format the S10 vanilla stand used).
    #[serde(default)]
    pub dev_geometry: Option<String>,
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
    /// only by the ignored parity tests of the `parity` crate, and refused as
    /// a `save_dir` root (Шаг 18a) so player files never land there.
    #[serde(default)]
    pub vanilla_server_dir: Option<String>,
    /// Optional directory the server saves player files in (Шаг 18a).
    ///
    /// Vanilla layout: `<save_dir>/players/data/<uuid>.dat`, one world per
    /// directory. Absent (the default) turns persistence **off**: nothing is
    /// written and every join starts at the spawn with 20/20/5 — a warning at
    /// startup says so. The path must not be the world template or the vanilla
    /// parity stand: the server refuses to start then, because player files
    /// must never land in a world the project ships or compares against. Each
    /// scenario run of `tools/testclient` points this at its own run
    /// directory.
    #[serde(default)]
    pub save_dir: Option<String>,
    /// Whether the configured `game_mode` wins over a saved `playerGameType`
    /// on join (Шаг 18a). Default `true`: the operator's configured mode is
    /// authoritative. Set it to `false` to let a returning player keep the
    /// mode its file carries (a file without a usable mode still gets the
    /// configured one).
    #[serde(default = "default_true")]
    pub force_gamemode: bool,
    /// Autosave period in ticks (Шаг 18a). Default 6000 (three minutes at
    /// 20 TPS). `0` disables the periodic save; a player quit and the server
    /// stop still save.
    #[serde(default = "default_autosave_ticks")]
    pub autosave_interval_ticks: u64,
    /// Autosave period of the **world** in ticks (Шаг 22b). Default 6000,
    /// independent of `autosave_interval_ticks`; `0` disables the periodic
    /// pass, while the save at the server stop still runs. Only chunks with
    /// unsaved block changes are written, at most 256 per pass, and the tick
    /// never waits for the disk.
    #[serde(default = "default_world_autosave_ticks")]
    pub world_autosave_ticks: u64,
    /// Apply the saved chunk diffs even when `world.json` records another
    /// template (another SHA-256 of `level.dat` or another `DataVersion`).
    ///
    /// Off by default: the server refuses to start on a mismatch, because a
    /// diff written against one world palette would silently produce a wrong
    /// world in another. Turning it on keeps the diffs and logs a warning —
    /// the operator's explicit choice (Шаг 22b, `docs/WORLD-SAVE.md` §6).
    #[serde(default)]
    pub save_ignore_world_mismatch: bool,
    /// Operators (Шаг 25b): player names or UUIDs that join with operator
    /// level 2. Empty by default, so nobody is an operator.
    ///
    /// A name is resolved to the offline UUID the login and the player file
    /// use; a UUID is taken as written. An entry that is neither a known
    /// name nor a UUID stops the startup (a typo must not silently grant or
    /// withhold rights).
    #[serde(default)]
    pub ops: Vec<String>,
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
            .collision_report
            .as_deref()
            .is_some_and(|path| path.trim().is_empty())
        {
            return Err(ConfigError::Invalid("collision_report is empty".to_owned()));
        }
        if self
            .hardness_report
            .as_deref()
            .is_some_and(|path| path.trim().is_empty())
        {
            return Err(ConfigError::Invalid("hardness_report is empty".to_owned()));
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
        if self
            .save_dir
            .as_deref()
            .is_some_and(|path| path.trim().is_empty())
        {
            return Err(ConfigError::Invalid("save_dir is empty".to_owned()));
        }
        if !(0..=3).contains(&self.game_mode) {
            return Err(ConfigError::Invalid(format!(
                "game_mode must be 0..=3 (survival, creative, adventure, spectator), got {}",
                self.game_mode
            )));
        }
        if let Some(value) = self.spawn_override.as_deref() {
            parse_spawn_override(value)?;
        }
        if self
            .dev_geometry
            .as_deref()
            .is_some_and(|path| path.trim().is_empty())
        {
            return Err(ConfigError::Invalid("dev_geometry is empty".to_owned()));
        }
        for entry in &self.ops {
            let entry = entry.trim();
            if entry.is_empty() {
                return Err(ConfigError::Invalid(
                    "ops contains an empty entry (a name or a UUID)".to_owned(),
                ));
            }
            if entry.contains('-') && parse_uuid(entry).is_none() {
                return Err(ConfigError::Invalid(format!(
                    "ops entry {entry:?} looks like a UUID but does not parse"
                )));
            }
            if parse_uuid(entry).is_none() && entry.chars().count() > 16 {
                return Err(ConfigError::Invalid(format!(
                    "ops entry {entry:?} is neither a UUID nor a player name of at most 16 characters"
                )));
            }
        }
        Ok(())
    }
}

/// Parses a UUID written as 32 hex digits, with or without hyphens, into the
/// 16 bytes the protocol uses (Шаг 25b, the `ops` list).
///
/// Only the canonical layout is accepted: 8-4-4-4-12 with hyphens or the
/// same digits without them.
pub(crate) fn parse_uuid(text: &str) -> Option<protocol::Uuid> {
    let text = text.trim();
    let hex: String = text.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    if text.contains('-') {
        let groups: Vec<&str> = text.split('-').collect();
        let lengths: Vec<usize> = groups.iter().map(|group| group.len()).collect();
        if lengths != [8, 4, 4, 4, 12] {
            return None;
        }
    }
    let mut uuid = [0u8; 16];
    for (index, byte) in uuid.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(uuid)
}

/// Parses the development-only integer `x,y,z` spawn tuple.
pub(crate) fn parse_spawn_override(value: &str) -> Result<protocol::Position, ConfigError> {
    let coordinates: Vec<_> = value.split(',').map(str::trim).collect();
    if coordinates.len() != 3 {
        return Err(ConfigError::Invalid(
            "spawn_override must contain exactly x,y,z integer coordinates".to_owned(),
        ));
    }
    let parse = |axis: &str, coordinate: &str| {
        coordinate.parse::<i32>().map_err(|_| {
            ConfigError::Invalid(format!(
                "spawn_override {axis} coordinate is not a 32-bit integer: {coordinate}"
            ))
        })
    };
    Ok(protocol::Position {
        x: parse("x", coordinates[0])?,
        y: parse("y", coordinates[1])?,
        z: parse("z", coordinates[2])?,
    })
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
    fn tick_thread_priority_defaults_off_and_parses_an_override() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert!(!config.tick_thread_priority, "absent means the default off");

        let text = example_toml() + "tick_thread_priority = true\n";
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert!(config.tick_thread_priority, "the flag is parsed");
        config.validate().expect("valid");
    }

    #[test]
    fn game_mode_defaults_to_survival_and_parses_an_override() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(config.game_mode, 0, "absent means survival");

        let text = example_toml() + "game_mode = 1\n";
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(config.game_mode, 1, "creative");
        config.validate().expect("valid");
    }

    #[test]
    fn world_save_settings_have_safe_defaults_and_parse_overrides() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(config.world_autosave_ticks, DEFAULT_AUTOSAVE_TICKS);
        assert!(
            !config.save_ignore_world_mismatch,
            "mismatch refuses by default"
        );

        let text =
            example_toml() + "world_autosave_ticks = 200\nsave_ignore_world_mismatch = true\n";
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(config.world_autosave_ticks, 200);
        assert!(config.save_ignore_world_mismatch);
        config.validate().expect("valid");
    }

    #[test]
    fn spawn_override_is_optional_and_parses_explicit_coordinates() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(config.spawn_override, None);

        let text = example_toml() + "spawn_override = \"106,41,-31\"\n";
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        config.validate().expect("valid override");
        assert_eq!(
            parse_spawn_override(config.spawn_override.as_deref().expect("override"))
                .expect("parsed"),
            protocol::Position {
                x: 106,
                y: 41,
                z: -31
            }
        );
    }

    #[test]
    fn rejects_malformed_spawn_override_coordinates() {
        for value in ["106,41", "106.5,41,-31", "106,41,999999999999"] {
            let text = example_toml() + &format!("spawn_override = \"{value}\"\n");
            let config: ServerConfig = toml::from_str(&text).expect("toml string");
            assert!(config.validate().is_err(), "accepted {value}");
        }
    }

    #[test]
    fn dev_geometry_is_optional_and_validated() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(config.dev_geometry, None, "absent means disabled");
        config.validate().expect("valid");

        let text = example_toml() + "dev_geometry = \"E:/data/s10-commands.txt\"\n";
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(
            config.dev_geometry.as_deref(),
            Some("E:/data/s10-commands.txt")
        );
        config.validate().expect("valid");

        let blank = example_toml() + "dev_geometry = \" \"\n";
        let config: ServerConfig = toml::from_str(&blank).expect("valid toml");
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn rejects_a_game_mode_outside_the_four_vanilla_modes() {
        for mode in [-1, 4] {
            let text = example_toml() + &format!("game_mode = {mode}\n");
            let config: ServerConfig = toml::from_str(&text).expect("valid toml");
            assert!(
                matches!(config.validate(), Err(ConfigError::Invalid(_))),
                "game_mode = {mode} must be rejected"
            );
        }
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
    fn collision_report_path_is_optional() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(
            config.collision_report, None,
            "absent means the forced pose is off"
        );

        let text = example_toml()
            + r#"collision_report = "generated/reports/collision.json"
"#;
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(
            config.collision_report.as_deref(),
            Some("generated/reports/collision.json")
        );
        config.validate().expect("valid");
    }

    #[test]
    fn rejects_an_empty_collision_report_path() {
        let text = example_toml()
            + r#"collision_report = " "
"#;
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn hardness_report_path_is_optional_and_validated() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(
            config.hardness_report, None,
            "absent means the mining timer trusts the client"
        );
        config.validate().expect("valid");

        let text = example_toml()
            + r#"hardness_report = "generated/reports/hardness.json"
"#;
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(
            config.hardness_report.as_deref(),
            Some("generated/reports/hardness.json")
        );
        config.validate().expect("valid");

        let empty = example_toml()
            + r#"hardness_report = " "
"#;
        let config: ServerConfig = toml::from_str(&empty).expect("valid toml");
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
    }

    /// Шаг 18.0.1: the finish-time tolerance is off unless the file sets it.
    #[test]
    fn dig_tolerance_ticks_defaults_to_none_and_parses_an_override() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert_eq!(
            config.dig_tolerance_ticks, None,
            "absent trusts the client about the finish"
        );
        config.validate().expect("valid");

        let text = example_toml() + "dig_tolerance_ticks = 3\n";
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        assert_eq!(config.dig_tolerance_ticks, Some(3));
        config.validate().expect("valid");
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
    fn ops_accept_names_and_uuids_and_refuse_a_broken_entry() {
        let config: ServerConfig = toml::from_str(&example_toml()).expect("valid toml");
        assert!(config.ops.is_empty(), "nobody is an operator by default");
        config.validate().expect("valid");

        let text = example_toml() + "ops = [\"Alice\", \"11111111-2222-3333-4444-555555555555\"]\n";
        let config: ServerConfig = toml::from_str(&text).expect("valid toml");
        config
            .validate()
            .expect("a name and a UUID are both accepted");
        assert_eq!(config.ops.len(), 2);

        let broken = example_toml() + "ops = [\"1111-2222\"]\n";
        let config: ServerConfig = toml::from_str(&broken).expect("valid toml");
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));

        let empty = example_toml() + "ops = [\"\"]\n";
        let config: ServerConfig = toml::from_str(&empty).expect("valid toml");
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn a_uuid_parses_with_and_without_hyphens() {
        let canonical = "11111111-2222-3333-4444-555555555555";
        let expected = parse_uuid(canonical).expect("canonical");
        assert_eq!(expected[0], 0x11);
        assert_eq!(expected[15], 0x55);
        assert_eq!(
            parse_uuid("11111111222233334444555555555555"),
            Some(expected)
        );
        assert_eq!(parse_uuid("11111111-2222-3333-4444-5555"), None);
        assert_eq!(parse_uuid("not-a-uuid"), None);
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
