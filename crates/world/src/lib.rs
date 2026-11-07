//! Reading the frozen world template: Anvil region files and the chunk model.
//!
//! This crate turns the `dimensions/<dimension>/region/r.X.Z.mca` files of a
//! generated world into a [`Chunk`] model and encodes nothing: sending a
//! chunk over the wire is the job of `protocol` (step 3.2), running the model
//! under load is the job of the loader (step 3.3).
//!
//! The read path follows the public Anvil layout (4 KiB location header,
//! length-prefixed zlib/gzip/raw sector) and the NBT specification; the exact
//! field set of the stored chunk was observed from the owner's own world
//! template (`crates/parity` step 3.0, `world_inspect`). Anything outside
//! that observation is an explicit error — a format we do not recognise is
//! never guessed at.

mod chunk;
mod error;
mod nbt;
mod region;
mod send;

pub use chunk::{Chunk, ChunkNotReady, Section};
pub use error::WorldError;
pub use region::{load_region, region_coords, ChunkPos, StoredChunk};
pub use send::{load_sendable_chunk, ChunkUnavailable};
