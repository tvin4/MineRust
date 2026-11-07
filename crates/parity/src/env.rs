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
    fn server_toml_defaults_to_the_workspace_root() {
        let path = server_toml_path();
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(path, manifest.join("..").join("..").join("server.toml"));
    }

    #[test]
    fn the_env_var_overrides_the_config_path() {
        // Env vars are process-global; the assertion still holds because the
        // only thing we check is that the variable wins.
        let var = SERVER_TOML_VAR;
        let previous = std::env::var(var).ok();
        std::env::set_var(var, "custom/server.toml");
        let path = server_toml_path();
        if let Some(previous) = previous {
            std::env::set_var(var, previous);
        } else {
            std::env::remove_var(var);
        }
        assert_eq!(path, PathBuf::from("custom/server.toml"));
    }
}
