//! The parity test environment: paths from the gitignored `server.toml`.
//!
//! Nothing here is stored in the repository: the vanilla `server.jar` is the
//! official Mojang build (licensed separately) and the world template is a
//! generated Minecraft world. See `server.example.toml` for the fields and
//! `docs/SOURCES.md` for the parity plan.

use std::path::{Path, PathBuf};

use server::{load_config, ConfigError, ServerConfig};

/// Overrides the default location of `server.toml`.
pub const SERVER_TOML_VAR: &str = "MINECRAFT_SERVER_TOML";

/// Environment errors: a missing/unreadable config or missing vanilla files.
#[derive(Debug, thiserror::Error)]
pub enum EnvError {
    /// The configuration file could not be loaded or validated.
    #[error("cannot load parity config {path}: {source}")]
    Config {
        /// Path that was read.
        path: String,
        /// Underlying configuration error (boxed: it is large).
        #[source]
        source: Box<ConfigError>,
    },
    /// The config exists but does not point at the vanilla installation.
    #[error(
        "parity needs `{field}` in server.toml (see server.example.toml); \
         run the vanilla setup from docs/PROGRESS.md first"
    )]
    NotConfigured {
        /// Name of the missing field.
        field: &'static str,
    },
    /// The vanilla server jar does not exist at the configured path.
    #[error("vanilla server jar not found: {path}")]
    MissingJar {
        /// Configured jar path.
        path: PathBuf,
    },
    /// The vanilla working directory does not exist.
    #[error("vanilla server dir not found: {path}")]
    MissingDir {
        /// Configured working directory.
        path: PathBuf,
    },
    /// The world template (a sibling of the vanilla dir) does not exist.
    #[error("world template not found: {path} (save a vanilla world there)")]
    MissingWorldTemplate {
        /// Expected template directory.
        path: PathBuf,
    },
    /// The vanilla `server.properties` could not be read.
    #[error("cannot read vanilla server.properties at {path}: {source}")]
    PropertiesRead {
        /// `server.properties` path.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// `server-port` is missing or not a number.
    #[error("server-port not found or invalid in {path}")]
    BadPort {
        /// `server.properties` path.
        path: PathBuf,
    },
    /// The configured packets report could not be loaded or verified.
    #[error("cannot load packets report {path}: {source}")]
    Report {
        /// Report path that failed.
        path: String,
        /// Underlying data error.
        #[source]
        source: Box<data::DataError>,
    },
}

/// Everything the parity tests need to talk to the real vanilla server.
#[derive(Debug, Clone)]
pub struct ParityEnv {
    /// The full `server.toml`, also used to start our own server.
    pub config: ServerConfig,
    /// Official vanilla `server.jar` (outside the repository).
    pub vanilla_jar: PathBuf,
    /// Vanilla working directory: `eula.txt`, `server.properties`, logs.
    pub vanilla_dir: PathBuf,
    /// `world-template` directory with a frozen survival world (seed 12345).
    pub world_template: PathBuf,
}

/// Environment variable with the packets report, as used by the data tests.
pub const PACKETS_REPORT_VAR: &str = "MINECRAFT_PACKETS_REPORT";

impl ParityEnv {
    /// The port vanilla listens on, read from its `server.properties`.
    pub fn vanilla_port(&self) -> Result<u16, EnvError> {
        let path = self.vanilla_dir.join("server.properties");
        let text = std::fs::read_to_string(&path).map_err(|source| EnvError::PropertiesRead {
            path: path.clone(),
            source,
        })?;
        parse_server_port(&text).ok_or(EnvError::BadPort { path })
    }

    /// The packet ids every side of a trace is keyed by.
    ///
    /// Uses the configured packets report (or `$MINECRAFT_PACKETS_REPORT`
    /// when the config field is empty) and fails if it does not cover our
    /// packet keys; without any report the compiled, report-verified
    /// defaults are used.
    pub fn packet_ids(&self) -> Result<data::PacketIds, EnvError> {
        if let Some(path) = self
            .config
            .packets_report
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var(PACKETS_REPORT_VAR).ok().map(PathBuf::from))
        {
            let display = path.display().to_string();
            let ids = data::load_packets_report(&path).map_err(|source| EnvError::Report {
                path: display.clone(),
                source: Box::new(source),
            })?;
            ids.verify(protocol::used_packet_keys())
                .map_err(|source| EnvError::Report {
                    path: display,
                    source: Box::new(source),
                })?;
            return Ok(ids);
        }
        tracing::warn!("parity: no packets report configured, using compiled packet ids");
        Ok(data::PacketIds::compiled())
    }
}

/// Extracts `server-port` from `server.properties` text.
fn parse_server_port(text: &str) -> Option<u16> {
    text.lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("server-port="))
        .and_then(|value| value.trim().parse().ok())
}

/// Locates `server.toml`: `$MINECRAFT_SERVER_TOML` or two levels above this
/// crate (the workspace root).
fn server_toml_path() -> PathBuf {
    if let Ok(path) = std::env::var(SERVER_TOML_VAR) {
        return PathBuf::from(path);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("server.toml")
}

/// Loads `server.toml` and resolves the vanilla installation paths.
///
/// Fails with [`EnvError::NotConfigured`] when the owner has not filled the
/// vanilla fields in yet; parity tests should print that error and skip.
pub fn load_parity_env() -> Result<ParityEnv, EnvError> {
    let path = server_toml_path();
    let path_display = path.display().to_string();
    let config = load_config(&path).map_err(|source| EnvError::Config {
        path: path_display,
        source: Box::new(source),
    })?;

    let vanilla_jar = PathBuf::from(
        config
            .vanilla_server_jar
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or(EnvError::NotConfigured {
                field: "vanilla_server_jar",
            })?,
    );
    let vanilla_dir = PathBuf::from(
        config
            .vanilla_server_dir
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or(EnvError::NotConfigured {
                field: "vanilla_server_dir",
            })?,
    );

    if !vanilla_jar.is_file() {
        return Err(EnvError::MissingJar { path: vanilla_jar });
    }
    if !vanilla_dir.is_dir() {
        return Err(EnvError::MissingDir { path: vanilla_dir });
    }
    // The template lives next to the vanilla install: `C:\MineRust-data\world-template`.
    let world_template = vanilla_dir
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| vanilla_dir.clone())
        .join("world-template");
    if !world_template.is_dir() {
        return Err(EnvError::MissingWorldTemplate {
            path: world_template,
        });
    }

    Ok(ParityEnv {
        config,
        vanilla_jar,
        vanilla_dir,
        world_template,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_port_is_read_from_the_properties_text() {
        let text = "# comment\nlevel-seed=12345\nserver-port=25566\nview-distance=10\n";
        assert_eq!(parse_server_port(text), Some(25566));
        assert_eq!(
            parse_server_port("server-port = 25565\n"),
            None,
            "no spaces"
        );
        assert_eq!(parse_server_port("level-seed=1\n"), None, "missing key");
        assert_eq!(parse_server_port("server-port=abc\n"), None, "not a number");
    }

    #[test]
    fn server_toml_path_defaults_to_the_workspace_root_and_honours_the_env() {
        // One sequential test: env vars are process-global, so a second
        // test changing the same variable could race this one.
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let default = manifest.join("..").join("..").join("server.toml");

        let var = SERVER_TOML_VAR;
        let previous = std::env::var(var).ok();
        std::env::remove_var(var);
        let path_without_env = server_toml_path();

        std::env::set_var(var, "custom/server.toml");
        let path_with_env = server_toml_path();

        match previous {
            Some(previous) => std::env::set_var(var, previous),
            None => std::env::remove_var(var),
        }

        assert_eq!(path_without_env, default);
        assert_eq!(path_with_env, PathBuf::from("custom/server.toml"));
    }
}
