//! The saved world behind the loader: id tables and the batch reader.
//!
//! Two things live here: [`WireTables`], the [`world::WireRefs`] lookups
//! binding stored palette names to the numeric ids of the wire, and
//! [`TemplateWorld`], the [`net::ChunkSource`] that turns the region files
//! of a world save into `level_chunk_with_light` payloads on the loader
//! pool (`net::loader`). Both are built once at startup by `run`; nothing
//! here is shared mutably with a connection.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use data::{BlockStates, GameData, RegistryIds};
use net::loader::LoadError;
use protocol::LevelChunkWithLight;
use tracing::{debug, info, warn};
use world::{Chunk, ChunkNotReady, StoredChunk};

/// Registry whose position defines a block entity type id.
const BLOCK_ENTITY_REGISTRY: &str = "minecraft:block_entity_type";

/// Registry whose packet position defines a biome id.
const BIOME_REGISTRY: &str = "minecraft:worldgen/biome";

/// The three report lookups [`world::WireRefs`] asks for, bound to the
/// data of the user's game copy.
///
/// Sources of the ids (step 3.3c, decided in `docs/PROGRESS.md`):
/// block states from `reports/blocks.json`, block entity types from
/// `reports/registries.json`, biome ids from the index in the
/// `minecraft:worldgen/biome` registry of the loaded game data (the
/// Registry Data packet numbers entries by their position).
pub struct WireTables {
    /// Block name (+properties) → state id (`blocks.json`).
    blocks: BlockStates,
    /// Built-in registry ids (`registries.json`).
    registries: RegistryIds,
    /// Loaded game data: synchronized registries and tags.
    game_data: Arc<GameData>,
}

impl WireTables {
    /// Binds the tables to the loaded reports; an absent report leaves its
    /// table empty and the conversion names the entry it could not resolve
    /// (`WireError::UnknownBlock`, no invented id).
    pub fn new(blocks: BlockStates, registries: RegistryIds, game_data: Arc<GameData>) -> Self {
        Self {
            blocks,
            registries,
            game_data,
        }
    }
}

impl world::WireRefs for WireTables {
    fn block_state_id(
        &self,
        name: &str,
        properties: Option<&std::collections::BTreeMap<String, String>>,
    ) -> Option<i32> {
        self.blocks.state_id(name, properties)
    }

    fn block_entity_type_id(&self, name: &str) -> Option<i32> {
        self.registries.entry_id(BLOCK_ENTITY_REGISTRY, name)
    }

    fn biome_id(&self, name: &str) -> Option<i32> {
        self.game_data
            .registry(BIOME_REGISTRY)
            .and_then(|registry| registry.entry_id(name))
    }
}

/// The ready chunks of one world save, read from its region files.
pub struct TemplateWorld {
    /// `dimensions/minecraft/overworld/region` of the save.
    region_dir: PathBuf,
    /// `Data.DataVersion` read from `level.dat` at startup.
    data_version: i32,
    /// Id lookups of the wire conversion.
    refs: Arc<WireTables>,
    /// The whole batch of ready payloads, built once and then shared by
    /// every connection (`Arc`, first build wins the `OnceLock` slot).
    ///
    /// The reader used to re-read every region file and re-encode every
    /// chunk on each job: the owner's log showed `world batch built
    /// sent=729` once at the warm-up and again on every join (~2.4 s of
    /// work). The cache turns every later job into an `Arc` clone: an
    /// immutable batch with no `Mutex` anywhere on the join path or in the
    /// tick (docs/DECISIONS.md rule: a new lock needs a written reason —
    /// this is the lock-free side of the answer instead).
    ready: OnceLock<Arc<Vec<LevelChunkWithLight>>>,
}

impl TemplateWorld {
    /// Points the reader at a save; `region_dir` is checked by `run`
    /// before anything binds a port.
    pub fn new(region_dir: PathBuf, data_version: i32, refs: Arc<WireTables>) -> Self {
        Self {
            region_dir,
            data_version,
            refs,
            ready: OnceLock::new(),
        }
    }

    /// All region files of the dimension, name-sorted so one batch has a
    /// deterministic send order (the recorded order still comes from the
    /// vanilla trace, reported by the 3.3f parity test).
    fn region_files(&self) -> Result<Vec<PathBuf>, LoadError> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&self.region_dir)? {
            let path = entry?.path();
            if path.extension().is_some_and(|ext| ext == "mca") {
                files.push(path);
            }
        }
        files.sort();
        Ok(files)
    }

    /// One stored chunk: the ready rule first, then the wire conversion.
    ///
    /// `Ok(None)` is the rule refusing the chunk by name — a chunk waiting
    /// for the generator (M6) is never sent. Every other failure is a data
    /// problem the caller must see (`LoadError`), never a silent default.
    fn payload_of(&self, stored: StoredChunk) -> Result<Option<LevelChunkWithLight>, LoadError> {
        let chunk = Chunk::from_stored(&stored)?;
        if let Err(reason) = chunk.check_sendable(self.data_version) {
            match &reason {
                ChunkNotReady::UnsupportedVersion { .. } => {
                    warn!(%reason, "chunk does not match level.dat")
                }
                ChunkNotReady::NotGenerated { .. } => debug!(%reason, "chunk waits for M6"),
                other => debug!(reason = %other, "chunk refused by the ready rule"),
            }
            return Ok(None);
        }
        let payload = chunk.level_chunk(self.refs.as_ref())?;
        Ok(Some(payload))
    }

    /// The shared batch: the first caller reads the world and stores it,
    /// every later caller only clones the `Arc`. No `Mutex` stands between
    /// a connection and a payload — the hand-over is `OnceLock`, and two
    /// racing first readers cost one wasted build at most, only while the
    /// cache is still empty (the warm-up fills it before any port binds).
    fn ready_batch(&self) -> Result<Arc<Vec<LevelChunkWithLight>>, LoadError> {
        if let Some(cached) = self.ready.get() {
            return Ok(Arc::clone(cached));
        }
        let built = Arc::new(self.build_batch()?);
        Ok(Arc::clone(self.ready.get_or_init(|| built)))
    }

    /// Reads every region file once and converts the chunks that pass the
    /// ready rule. Region files are read as they are listed: one broken
    /// file fails the whole batch with its path in the error (a malformed
    /// world is a read error, never a hole in the terrain).
    ///
    /// The `world batch built` summary belongs here and only here: it used
    /// to print on every join — around 2.4 s of reading and encoding that
    /// this cache now does once at warm-up.
    fn build_batch(&self) -> Result<Vec<LevelChunkWithLight>, LoadError> {
        let files = self.region_files()?;
        let mut payload = Vec::new();
        let mut refused = 0_usize;
        for path in files {
            for stored in world::load_region(&path)? {
                match self.payload_of(stored)? {
                    Some(chunk) => payload.push(chunk),
                    None => refused += 1,
                }
            }
        }
        info!(
            sent = payload.len(),
            refused, self.data_version, "world batch built"
        );
        Ok(payload)
    }
}

impl net::ChunkSource for TemplateWorld {
    /// Every ready chunk of the save as a wire payload behind the shared
    /// `Arc`: the first call builds the batch (the warm-up owns it), later
    /// calls return the same immutable batch out of [`Self::ready_batch`].
    /// A join clones the pointer — never the payloads.
    fn load_batch(&self) -> Result<Arc<Vec<LevelChunkWithLight>>, LoadError> {
        self.ready_batch()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use net::ChunkSource as _;

    /// Part A 2: the first read owns the disk, every later read is served
    /// from the immutable cache. The proof is the missing directory — a
    /// rebuild would fail with an I/O error here, the cached batch does
    /// not notice that the world it came from is gone.
    #[test]
    fn the_batch_is_built_once_and_then_served_from_the_cache() {
        let region_dir = std::env::temp_dir().join("minerust-batch-cache");
        let _ = std::fs::remove_dir_all(&region_dir);
        std::fs::create_dir_all(&region_dir).expect("region dir");
        let world = TemplateWorld::new(
            region_dir.clone(),
            5023,
            Arc::new(WireTables::new(
                BlockStates::default(),
                RegistryIds::default(),
                Arc::new(GameData::default()),
            )),
        );

        let first = world.load_batch().expect("first read of the empty save");
        assert!(first.is_empty(), "an empty region holds no chunks");

        // The disk goes away: without the cache this would be an error.
        std::fs::remove_dir_all(&region_dir).expect("region dir removed");
        let second = world.load_batch().expect("second read from the cache");
        assert_eq!(second.len(), first.len(), "the very same batch");

        let cached = world.ready.get().expect("the batch was stored");
        assert_eq!(cached.len(), first.len(), "the cache holds the batch");
        let _ = std::fs::remove_dir_all(&region_dir);
    }
}
