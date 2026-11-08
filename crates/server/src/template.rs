//! The saved world behind the loader: id tables and the batch reader.
//!
//! Two things live here: [`WireTables`], the [`world::WireRefs`] lookups
//! binding stored palette names to the numeric ids of the wire, and
//! [`TemplateWorld`], the [`net::ChunkSource`] that turns the region files
//! of a world save into `level_chunk_with_light` payloads on the loader
//! pool (`net::loader`). Both are built once at startup by `run`; nothing
//! here is shared mutably with a connection.

use std::path::PathBuf;
use std::sync::Arc;

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
}

impl TemplateWorld {
    /// Points the reader at a save; `region_dir` is checked by `run`
    /// before anything binds a port.
    pub fn new(region_dir: PathBuf, data_version: i32, refs: Arc<WireTables>) -> Self {
        Self {
            region_dir,
            data_version,
            refs,
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
}

impl net::ChunkSource for TemplateWorld {
    /// Every chunk of the save that passes the ready rule, converted to a
    /// wire payload. Region files are read as they are listed: one broken
    /// file fails the whole batch with its path in the error (a malformed
    /// world is a read error, never a hole in the terrain).
    fn load_batch(&self) -> Result<Vec<LevelChunkWithLight>, LoadError> {
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
