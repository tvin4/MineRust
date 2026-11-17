//! Errors of the playerdata store.

use std::io;

/// Everything that can go wrong while reading or writing a player file.
#[derive(Debug, thiserror::Error)]
pub enum PlayerdataError {
    /// The filesystem rejected an operation on `path`.
    #[error("playerdata I/O at {path}: {source}")]
    Io {
        /// Path the operation touched.
        path: String,
        /// Underlying I/O error.
        source: io::Error,
    },
    /// The file is not a gzip-compressed NBT compound with the expected
    /// fields.
    #[error("playerdata document at {path} is not usable: {detail}")]
    Corrupt {
        /// Path of the document.
        path: String,
        /// What was wrong with it.
        detail: String,
    },
    /// `save_dir` resolves to a directory the server must never write into
    /// (the world template or the vanilla parity stand).
    #[error("save_dir {save_dir} is {relation} the forbidden {label} directory {forbidden}")]
    ForbiddenPath {
        /// Configured save directory.
        save_dir: String,
        /// Either "equal to" or "inside".
        relation: &'static str,
        /// What the directory is (`world template`, `vanilla stand`).
        label: &'static str,
        /// Forbidden directory it collided with.
        forbidden: String,
    },
    /// A value cannot be stored: it is not a finite number or outside the
    /// range the vanilla format uses.
    #[error("playerdata value {field} is invalid: {detail}")]
    InvalidValue {
        /// NBT key the value would be written to.
        field: &'static str,
        /// What is wrong with it.
        detail: String,
    },
    /// The writer thread did not finish within the shutdown timeout.
    #[error("the playerdata writer did not stop within the timeout")]
    ShutdownTimeout,
    /// The writer thread ended before the message could be delivered (it
    /// panicked).
    #[error("the playerdata writer is gone")]
    WriterGone,
    /// The writer thread could not be started.
    #[error("could not start the playerdata writer thread: {source}")]
    Spawn {
        /// Underlying thread-spawn error.
        source: io::Error,
    },
}

impl PlayerdataError {
    /// Wraps an I/O error with the path it happened on.
    pub(crate) fn io(path: impl std::fmt::Display, source: io::Error) -> Self {
        Self::Io {
            path: path.to_string(),
            source,
        }
    }

    /// Wraps a parse problem with the path of the document.
    pub(crate) fn corrupt(path: impl std::fmt::Display, detail: impl Into<String>) -> Self {
        Self::Corrupt {
            path: path.to_string(),
            detail: detail.into(),
        }
    }
}
