//! The world-save seam between the game thread and the writer thread
//! (Шаг 22b).
//!
//! The game thread owns the store and the dirty map; the writer owns the
//! disk. The seam is deliberately two methods: [`WorldSave::submit`] hands a
//! snapshot over without blocking (a full queue is `false`, never a wait) and
//! [`WorldSave::completed`] returns the acknowledgements that arrived since
//! the last autosave. The concrete implementation is `worldsave`'s runtime;
//! `net` only needs this trait, so the crate graph stays
//! `server → worldsave → net`.

use crate::store::{ChunkSave, ChunkSaved};

/// What the game loop needs from a world-save writer.
pub trait WorldSave: std::fmt::Debug + Send + Sync {
    /// Queues the dirty chunks. Never blocks: `false` means the queue was
    /// full (or the writer is gone) and the chunks stay dirty for the next
    /// pass.
    fn submit(&self, chunks: Vec<ChunkSave>) -> bool;

    /// The writer's reports since the last call; never blocks.
    fn completed(&self) -> Vec<ChunkSaved>;
}

/// How many chunks one autosave pass hands to the writer (Шаг 22b); the rest
/// keeps waiting for the next pass, so the queue cannot grow without bound.
pub const SAVE_CHUNKS_PER_PASS: usize = 256;
