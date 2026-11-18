//! The save root: identity file, diff files, and their application to the
//! template chunks (Шаг 22b).
//!
//! One directory holds everything: `world.json` (see [`crate::meta`]) and
//! `chunks/<x>.<z>.mrchunk`, one file per chunk whose cells differ from the
//! template. A chunk whose cells returned to the template values has no file;
//! the writer deletes it instead of storing an empty document.
//!
//! Reading happens once, at startup, before `warm_world` and before the port
//! is bound: [`Store::apply`] clones only the template chunks that carry a
//! diff and replaces their changed cells, and returns the baselines the store
//! needs to keep diffs relative to the template during the run.
//!
//! SOURCE: the owner's decision of Шаг 22b (`docs/DECISIONS.md`,
//! `docs/WORLD-SAVE.md`): diffs over the template, a file per chunk, a version
//! byte from the first day.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use data::BlockStates;
use net::store::{set_chunk_cell, SavedChunk};
use protocol::LevelChunkWithLight;
use thiserror::Error;
use tracing::{info, warn};

use crate::format::{decode, encode, ChunkDiff, FormatError};
use crate::meta::{self, MetaError, WorldIdentity, WorldMeta};

/// Subdirectory of the save root holding the chunk diffs.
pub const CHUNKS_DIR: &str = "chunks";

/// Extension of one chunk diff.
pub const CHUNK_EXTENSION: &str = "mrchunk";

/// Why the save root cannot be opened, read or written.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The identity file refused the start or could not be written.
    #[error(transparent)]
    Meta(#[from] MetaError),
    /// A directory, file or rename failed.
    #[error("could not {action} {path}: {source}")]
    Io {
        /// What was attempted (`create`, `read`, `write`, `rename`, ...).
        action: &'static str,
        /// Path involved.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A chunk file is not a valid document.
    #[error("{path} is not a readable chunk diff: {source}")]
    Format {
        /// Path of the file.
        path: String,
        /// Decoder message.
        #[source]
        source: FormatError,
    },
    /// The writer thread could not be started.
    #[error("could not start the world-save writer: {source}")]
    Spawn {
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The writer thread panicked.
    #[error("the world-save writer is gone")]
    WriterGone,
    /// The writer did not drain its queue in time.
    #[error("the world-save writer did not stop in time")]
    ShutdownTimeout,
    /// The writer queue or final flush did not complete before its deadline.
    #[error("the world-save writer did not {action} in time")]
    WriterTimeout {
        /// Operation that reached its deadline (`queue the final flush`, ...).
        action: &'static str,
    },
}

/// An opened save root.
///
/// Cheap to clone (two paths and the parsed identity), so the writer thread
/// owns its own copy and the startup code keeps one for the diffs' baselines.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
    chunks: PathBuf,
    meta: WorldMeta,
    mismatch: bool,
}

impl Store {
    /// Opens (or creates) the save root for `identity`.
    ///
    /// `ignore_mismatch` follows the owner's switch of the same name in
    /// `server.toml`: `false` refuses to start when the recorded template
    /// differs, `true` keeps the diffs and logs a warning.
    pub fn open(
        root: impl AsRef<Path>,
        identity: &WorldIdentity,
        ignore_mismatch: bool,
    ) -> Result<Self, StoreError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(|source| StoreError::Io {
            action: "create",
            path: root.display().to_string(),
            source,
        })?;
        sync_parent(&root).map_err(|source| StoreError::Io {
            action: "sync parent of",
            path: root.display().to_string(),
            source,
        })?;
        let opened = meta::ensure(&root, identity, ignore_mismatch)?;
        let chunks = root.join(CHUNKS_DIR);
        fs::create_dir_all(&chunks).map_err(|source| StoreError::Io {
            action: "create",
            path: chunks.display().to_string(),
            source,
        })?;
        sync_parent(&chunks).map_err(|source| StoreError::Io {
            action: "sync parent of",
            path: chunks.display().to_string(),
            source,
        })?;
        info!(
            root = %root.display(),
            template = %opened.meta.template,
            mismatch = opened.mismatch,
            "world save opened"
        );
        Ok(Self {
            root,
            chunks,
            meta: opened.meta,
            mismatch: opened.mismatch,
        })
    }

    /// The save root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The parsed identity file.
    pub fn meta(&self) -> &WorldMeta {
        &self.meta
    }

    /// Whether the save belongs to another template and the override is on.
    pub fn mismatch(&self) -> bool {
        self.mismatch
    }

    /// Path of one chunk's diff file.
    pub fn chunk_path(&self, chunk_x: i32, chunk_z: i32) -> PathBuf {
        self.chunks
            .join(format!("{chunk_x}.{chunk_z}.{CHUNK_EXTENSION}"))
    }

    /// Reads one chunk diff; `None` when the chunk has no file.
    ///
    /// A damaged file is an error here; [`Self::apply`] turns it into a
    /// warning and leaves the chunk as the template has it.
    pub fn read_diff(&self, chunk_x: i32, chunk_z: i32) -> Result<Option<ChunkDiff>, StoreError> {
        let path = self.chunk_path(chunk_x, chunk_z);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(StoreError::Io {
                    action: "read",
                    path: path.display().to_string(),
                    source,
                })
            }
        };
        let diff = decode(&bytes).map_err(|source| StoreError::Format {
            path: path.display().to_string(),
            source,
        })?;
        if (diff.chunk_x, diff.chunk_z) != (chunk_x, chunk_z) {
            return Err(StoreError::Format {
                path: path.display().to_string(),
                source: FormatError::Coordinate {
                    found_x: diff.chunk_x,
                    found_z: diff.chunk_z,
                    expected_x: chunk_x,
                    expected_z: chunk_z,
                },
            });
        }
        Ok(Some(diff))
    }

    /// Writes one chunk diff: empty means the chunk is back at the template,
    /// so the file is removed.
    ///
    /// The document goes to `<name>.tmp`, is flushed and synced, and is then
    /// renamed over the target, so a crash leaves either the old file or the
    /// new one, never a torn one.
    pub fn write_diff(&self, diff: &ChunkDiff) -> Result<(), StoreError> {
        let path = self.chunk_path(diff.chunk_x, diff.chunk_z);
        if diff.is_empty() {
            return match fs::remove_file(&path) {
                Ok(()) => sync_parent(&path).map_err(|source| StoreError::Io {
                    action: "sync after removing",
                    path: path.display().to_string(),
                    source,
                }),
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                    // A previous attempt may have removed the file and failed
                    // only while syncing this directory. Sync again before
                    // acknowledging the deletion.
                    sync_parent(&path).map_err(|source| StoreError::Io {
                        action: "sync after removing",
                        path: path.display().to_string(),
                        source,
                    })
                }
                Err(source) => Err(StoreError::Io {
                    action: "remove",
                    path: path.display().to_string(),
                    source,
                }),
            };
        }
        let temporary = temporary_path(&path);
        let bytes = encode(diff);
        let result = (|| -> std::io::Result<()> {
            let mut file = fs::File::create(&temporary)?;
            file.write_all(&bytes)?;
            file.flush()?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, &path)?;
            sync_parent(&path)?;
            Ok(())
        })();
        result.map_err(|source| {
            let _ = fs::remove_file(&temporary);
            StoreError::Io {
                action: "write",
                path: path.display().to_string(),
                source,
            }
        })
    }

    /// Applies every readable diff to `chunks` and returns their baselines.
    ///
    /// `chunks` are the template payloads; only the ones with a diff are
    /// cloned, and only their changed cells are rewritten. A file that does
    /// not decode, names a chunk the template does not have, or carries a
    /// state id the block report does not know is skipped with a warning and
    /// kept on disk: the world falls back to the template for that chunk
    /// rather than starting with a half-applied diff.
    ///
    /// When [`Self::mismatch`] is set the operator already chose to keep the
    /// diffs (`save_ignore_world_mismatch = true`), so they are applied here;
    /// the warning was logged by [`Store::open`].
    pub fn apply(
        &self,
        chunks: &mut [Arc<LevelChunkWithLight>],
        blocks: &BlockStates,
    ) -> Result<Vec<SavedChunk>, StoreError> {
        let files = self.diff_files()?;
        let mut out = Vec::with_capacity(files.len());
        for path in files {
            let Some((chunk_x, chunk_z)) = parse_name(&path) else {
                warn!(path = %path.display(), "not a chunk diff file name; skipped");
                continue;
            };
            let diff = match self.read_diff(chunk_x, chunk_z) {
                Ok(Some(diff)) => diff,
                Ok(None) => continue,
                Err(err @ StoreError::Format { .. }) => {
                    warn!(%err, "skipped a damaged chunk diff");
                    continue;
                }
                Err(err) => return Err(err),
            };
            if diff.changes.is_empty() {
                continue;
            }
            let Some(index) = chunks
                .iter()
                .position(|chunk| (chunk.chunk_x, chunk.chunk_z) == (chunk_x, chunk_z))
            else {
                warn!(
                    chunk_x,
                    chunk_z, "the saved chunk is not in the template; skipped"
                );
                continue;
            };
            let baseline = Arc::clone(&chunks[index]);
            let mut applied = (*baseline).clone();
            let mut failure = None;
            for &(cell, state_id) in &diff.changes {
                if blocks.block_name(state_id).is_none() {
                    failure = Some(format!("state id {state_id} is not in the block report"));
                    break;
                }
                if let Err(err) = set_chunk_cell(&mut applied, cell, state_id, blocks) {
                    failure = Some(err.to_string());
                    break;
                }
            }
            if let Some(detail) = failure {
                warn!(
                    chunk_x,
                    chunk_z,
                    detail = %detail,
                    "the saved chunk diff does not fit the template; skipped"
                );
                continue;
            }
            info!(
                chunk_x,
                chunk_z,
                cells = diff.changes.len(),
                "applied a saved chunk diff"
            );
            chunks[index] = Arc::new(applied);
            out.push(SavedChunk {
                chunk_x,
                chunk_z,
                baseline,
                cells: diff.changes,
            });
        }
        Ok(out)
    }

    /// Every `*.mrchunk` file under `chunks/`, sorted by name so two runs
    /// apply and log them in the same order.
    fn diff_files(&self) -> Result<Vec<PathBuf>, StoreError> {
        let entries = fs::read_dir(&self.chunks).map_err(|source| StoreError::Io {
            action: "list",
            path: self.chunks.display().to_string(),
            source,
        })?;
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| StoreError::Io {
                action: "list",
                path: self.chunks.display().to_string(),
                source,
            })?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == CHUNK_EXTENSION) {
                files.push(path);
            }
        }
        files.sort();
        Ok(files)
    }
}

/// `<path>.tmp`, next to the target so the rename stays on one volume.
fn temporary_path(path: &Path) -> PathBuf {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    PathBuf::from(temporary)
}

/// Reads `<x>.<z>.mrchunk`; `None` for any other name.
fn parse_name(path: &Path) -> Option<(i32, i32)> {
    let stem = path.file_stem()?.to_str()?;
    let (x, z) = stem.split_once('.')?;
    Some((x.parse().ok()?, z.parse().ok()?))
}

/// Syncs the directory entry after a rename where the platform needs it.
///
/// Windows journals the rename in NTFS; on Unix a fresh name survives a crash
/// only after the directory itself is synced.
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
mod tests;
