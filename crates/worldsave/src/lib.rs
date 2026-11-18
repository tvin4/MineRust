//! Chunk-diff world save for the MineRust server (Шаг 22b).
//!
//! The served world comes from a template the server never writes; every
//! block change lives in memory (`net::store::ChunkStore`) and is saved as a
//! per-chunk diff against that template:
//!
//! - [`format`] — the `.mrchunk` document (magic, version, cells, CRC-32);
//! - [`meta`] — `world.json`, which pins the template's `level.dat`;
//! - [`Store`] — the save root: reads the diffs at startup, writes one chunk
//!   atomically;
//! - [`writer`] — the thread that owns the disk while the tick owns the
//!   world.
//!
//! The concurrency scheme and the format decisions are recorded in
//! `docs/DECISIONS.md` («Шаг 22b») and `docs/WORLD-SAVE.md`; this crate
//! implements them and adds no rules of its own.

pub mod format;
pub mod meta;
mod store;
mod writer;

pub use format::{ChunkDiff, FormatError, CELLS_PER_CHUNK, MAGIC, VERSION};
pub use meta::{MetaError, WorldIdentity, WorldMeta, GENERATOR, WORLD_FILE};
pub use store::{Store, StoreError, CHUNKS_DIR, CHUNK_EXTENSION};
pub use writer::{spawn, Runtime, Writer, DEFAULT_CAPACITY};
