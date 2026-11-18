//! `world.json`: which world a `save_dir` belongs to (Шаг 22b).
//!
//! A save holds diffs against a **template**; applying them to another
//! template would silently produce a wrong world. The file therefore pins
//! the template path, the SHA-256 of its `level.dat` and its `DataVersion`.
//! A mismatch refuses the start unless `save_ignore_world_mismatch = true`;
//! then the diffs are applied with a warning (the operator's explicit
//! choice, `docs/WORLD-SAVE.md` §6).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

/// Identity file of a world save.
pub const WORLD_FILE: &str = "world.json";

/// Value of the `generator` field this version writes.
pub const GENERATOR: &str = "minerust-world-save/1";

/// What the served template is, as the save must record it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldIdentity {
    /// Template path as configured (`world_dir`).
    pub template: String,
    /// Lowercase hex SHA-256 of `<template>/level.dat`.
    pub level_dat_sha256: String,
    /// `DataVersion` read from that `level.dat`.
    pub data_version: i32,
}

/// The parsed [`WORLD_FILE`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldMeta {
    /// Template the diffs were written against.
    pub template: String,
    /// SHA-256 of that template's `level.dat`.
    pub level_dat_sha256: String,
    /// `DataVersion` of that template.
    pub data_version: i32,
    /// Writer that produced the file.
    pub generator: String,
}

/// Why the identity file cannot be used.
#[derive(Debug, Error)]
pub enum MetaError {
    /// The file could not be read or written.
    #[error("could not {action} {path}: {source}")]
    Io {
        /// What was attempted (`read`, `write`).
        action: &'static str,
        /// Path involved.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The file is not a [`WorldMeta`] document.
    #[error("{path} is not a valid world.json: {detail}")]
    Json {
        /// Path involved.
        path: String,
        /// Parser message.
        detail: String,
    },
    /// The save belongs to another world and the override is off.
    #[error(
        "the save belongs to another world: {field} is {found}, the served template has {expected} \
         (set save_ignore_world_mismatch = true to apply the diffs anyway)"
    )]
    Mismatch {
        /// Field that differs.
        field: &'static str,
        /// Value of the served template.
        expected: String,
        /// Value recorded in the save.
        found: String,
    },
}

/// The identity file after opening: its contents and whether it differs from
/// the served template (only reachable with the override on).
#[derive(Debug, Clone)]
pub struct Opened {
    /// Parsed file.
    pub meta: WorldMeta,
    /// Whether [`WorldIdentity`] and [`WorldMeta`] disagree.
    pub mismatch: bool,
}

/// Reads [`WORLD_FILE`], or creates it from `identity` when absent.
///
/// `ignore_mismatch` turns a differing identity into a warning instead of
/// [`MetaError::Mismatch`]; the returned flag records that the caller is
/// applying the diffs only because the operator explicitly allowed it.
pub fn ensure(
    root: &Path,
    identity: &WorldIdentity,
    ignore_mismatch: bool,
) -> Result<Opened, MetaError> {
    let path = root.join(WORLD_FILE);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            let meta = WorldMeta {
                template: identity.template.clone(),
                level_dat_sha256: identity.level_dat_sha256.clone(),
                data_version: identity.data_version,
                generator: GENERATOR.to_owned(),
            };
            write_atomically(&path, &meta)?;
            info!(
                path = %path.display(),
                template = %meta.template,
                data_version = meta.data_version,
                "world identity written"
            );
            return Ok(Opened {
                meta,
                mismatch: false,
            });
        }
        Err(source) => {
            return Err(MetaError::Io {
                action: "read",
                path: path.display().to_string(),
                source,
            })
        }
    };
    let meta: WorldMeta = serde_json::from_slice(&bytes).map_err(|err| MetaError::Json {
        path: path.display().to_string(),
        detail: err.to_string(),
    })?;
    // Also covers retry after a previous run renamed the file successfully
    // but failed while syncing the directory entry.
    sync_parent(&path).map_err(|source| MetaError::Io {
        action: "sync",
        path: path.display().to_string(),
        source,
    })?;
    let field = if meta.level_dat_sha256 != identity.level_dat_sha256 {
        Some((
            "level_dat_sha256",
            &identity.level_dat_sha256,
            &meta.level_dat_sha256,
        ))
    } else if meta.data_version != identity.data_version {
        Some((
            "data_version",
            &identity.data_version.to_string(),
            &meta.data_version.to_string(),
        ))
    } else {
        None
    };
    let Some((field, expected, found)) = field else {
        return Ok(Opened {
            meta,
            mismatch: false,
        });
    };
    if !ignore_mismatch {
        return Err(MetaError::Mismatch {
            field,
            expected: expected.clone(),
            found: found.clone(),
        });
    }
    warn!(
        path = %path.display(),
        field,
        expected = %expected,
        found = %found,
        "the save belongs to another template; applying the diffs anyway"
    );
    Ok(Opened {
        meta,
        mismatch: true,
    })
}

/// Writes the identity through a synced temporary sibling and a rename.
fn write_atomically(path: &Path, meta: &WorldMeta) -> Result<(), MetaError> {
    let temporary = temporary_path(path);
    let document = serde_json::to_vec_pretty(meta).map_err(|err| MetaError::Json {
        path: path.display().to_string(),
        detail: err.to_string(),
    })?;
    let result = (|| -> Result<(), std::io::Error> {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(&document)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        sync_parent(path)
    })();
    result.map_err(|source| {
        let _ = fs::remove_file(&temporary);
        MetaError::Io {
            action: "write",
            path: path.display().to_string(),
            source,
        }
    })
}

/// `<path>.tmp`, next to the target so the rename stays on one volume.
fn temporary_path(path: &Path) -> PathBuf {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    PathBuf::from(temporary)
}

/// Makes the renamed identity directory entry durable on Unix. Windows
/// journals the rename and does not allow opening a directory for `fsync`.
fn sync_parent(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::File::open(dir)?.sync_all()?;
        }
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "minerust-worldsave-meta-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn identity(hash: &str, version: i32) -> WorldIdentity {
        WorldIdentity {
            template: "E:/worlds/template".to_owned(),
            level_dat_sha256: hash.to_owned(),
            data_version: version,
        }
    }

    /// The first open records the identity; a matching later open accepts it.
    #[test]
    fn the_identity_is_written_once_and_reused() {
        let dir = temp_dir("write");
        let identity = identity("abc", 5023);
        let opened = ensure(&dir, &identity, false).expect("first open");
        assert!(!opened.mismatch);
        assert_eq!(opened.meta.template, identity.template);
        assert_eq!(opened.meta.generator, GENERATOR);
        let path = dir.join(WORLD_FILE);
        assert!(path.is_file());
        assert!(
            !temporary_path(&path).exists(),
            "the temporary file is gone"
        );

        ensure(&dir, &identity, false).expect("same world");
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
        assert_eq!(json["generator"], GENERATOR);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Another `level.dat` refuses the start unless the override is on.
    #[test]
    fn another_template_is_refused_unless_overridden() {
        let dir = temp_dir("mismatch");
        ensure(&dir, &identity("abc", 5023), false).expect("first open");
        let other = identity("def", 5023);
        let error = ensure(&dir, &other, false).expect_err("refused");
        assert!(matches!(
            error,
            MetaError::Mismatch {
                field: "level_dat_sha256",
                ..
            }
        ));
        let opened = ensure(&dir, &other, true).expect("override");
        assert!(opened.mismatch, "the caller skips or applies the diffs");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A changed `DataVersion` is a mismatch too: the palette meaning may
    /// have changed even when the bytes of `level.dat` did not.
    #[test]
    fn another_data_version_is_a_mismatch() {
        let dir = temp_dir("version");
        ensure(&dir, &identity("abc", 5023), false).expect("first open");
        let error = ensure(&dir, &identity("abc", 5024), false).expect_err("refused");
        assert!(matches!(
            error,
            MetaError::Mismatch {
                field: "data_version",
                ..
            }
        ));
        let _ = fs::remove_dir_all(&dir);
    }

    /// A damaged document is named, not guessed.
    #[test]
    fn a_damaged_identity_is_an_error() {
        let dir = temp_dir("damaged");
        fs::write(dir.join(WORLD_FILE), b"{").expect("seed");
        let error = ensure(&dir, &identity("abc", 5023), false).expect_err("refused");
        assert!(matches!(error, MetaError::Json { .. }));
        let _ = fs::remove_dir_all(&dir);
    }
}
