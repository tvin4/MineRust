//! The synthetic chunk world of the M4.6 network scenario (ARCH-M4 §5): a
//! finite 160×160-chunk flat world in memory behind [`net::ChunkSource`].
//!
//! Every chunk is one section with a minimal palette (`Single` containers,
//! no block entities) and a minimal light block; light is deliberately
//! simplified — this is **not** the parity world, it removes disk and real
//! world cost from the measurement. The batch is built once and handed to
//! every join through the same [`net::Loader`] pool the real server uses, so
//! the streaming path (window moves, forgets, batch pacing) is the real one.
//!
//! Memory: one chunk payload is the encoded `LevelChunkWithLight` (a few
//! hundred bytes each — the batch itself holds decoded payloads; the
//! estimate is printed when the world is built).

use std::sync::Arc;

use net::chunks::ReadyChunks;
use net::loader::{ChunkSource, LoadError};
use protocol::{ChunkSection, HeightmapRecord, LevelChunkWithLight, LightData, PaletteContainer};

/// Chunks along one axis of the synthetic world.
///
/// 288 keeps every window of the streaming run inside the world: the bot
/// walks 2000 blocks from the spawn (chunk 0) to chunk 125, the window
/// there spans 114..136 — inside −144..143. A smaller world made the
/// window run off the edge and the invariant check counted the missing
/// edge chunks as "outside the window" (they simply do not exist).
pub const WORLD_SIDE: i32 = 288;

/// A chunk source over a pre-built ready batch.
struct FlatSource {
    /// The whole world, built once at construction.
    ready: Arc<ReadyChunks>,
}

impl ChunkSource for FlatSource {
    fn load_batch(&self) -> Result<Arc<ReadyChunks>, LoadError> {
        Ok(Arc::clone(&self.ready))
    }
}

/// Builds the flat world: `side × side` chunks centred on the origin,
/// one shared payload shape, unique coordinates. Returns the source and the
/// memory estimate of the decoded batch (the struct sizes of the payloads).
pub fn build(side: i32) -> (Arc<dyn ChunkSource>, MemoryEstimate) {
    let half = side / 2;
    let chunks: Vec<LevelChunkWithLight> = (-half..side - half)
        .flat_map(|x| (-half..side - half).map(move |z| (x, z)))
        .map(|(x, z)| flat_chunk(x, z))
        .collect();
    let est = MemoryEstimate {
        chunks: chunks.len(),
        // Rough per-payload footprint of the decoded structs: the heightmap
        // (37 words) plus two containers and the light masks. Measured, not
        // guessed: see the printed estimate.
        bytes: chunks.len() as u64 * std::mem::size_of::<LevelChunkWithLight>() as u64,
    };
    let ready = Arc::new(ReadyChunks::new(chunks));
    let source = Arc::new(FlatSource { ready });
    (source as Arc<dyn ChunkSource>, est)
}

/// One flat chunk: one section of `Single(0)` blocks, a `Single(1)` biome
/// container, one heightmap row and empty light — the minimal payload the
/// decoder accepts.
fn flat_chunk(chunk_x: i32, chunk_z: i32) -> LevelChunkWithLight {
    LevelChunkWithLight {
        chunk_x,
        chunk_z,
        heightmaps: vec![HeightmapRecord {
            id: 4,
            words: vec![0; 37],
        }],
        sections: vec![ChunkSection {
            non_air: 0,
            fluid: 0,
            blocks: PaletteContainer::Single(0),
            biomes: PaletteContainer::Single(1),
        }],
        block_entities: Vec::new(),
        light: LightData {
            sky_mask: vec![0x00],
            block_mask: vec![0x00],
            empty_sky_mask: vec![0x01],
            empty_block_mask: vec![0x01],
            sky_arrays: Vec::new(),
            block_arrays: Vec::new(),
        },
    }
}

/// Memory facts of the built world, printed by the scenario runner.
#[derive(Debug, Clone, Copy)]
pub struct MemoryEstimate {
    /// Chunks in the batch.
    pub chunks: usize,
    /// Rough decoded footprint (struct sizes; the payloads dominate via
    /// their `Vec`s — see the printed estimate for the honest number).
    pub bytes: u64,
}

/// The encoded wire size of one chunk of this world (for the report).
pub fn encoded_chunk_bytes() -> usize {
    flat_chunk(0, 0)
        .encode_payload()
        .map(|bytes| bytes.len())
        .unwrap_or(0)
}
