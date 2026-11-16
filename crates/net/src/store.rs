//! The shared block store behind the wire world (Шаг 13a).
//!
//! One [`ChunkStore`] holds the current immutable snapshot of every ready
//! `level_chunk_with_light` payload ([`ReadyChunks`]) plus the block report
//! the section headers read ([`data::BlockStates`]). It is the single piece
//! of world state shared between the connections that send chunks and the
//! game thread that changes blocks: the ownership and concurrency scheme is
//! written down in `docs/DECISIONS.md` («Шаг 13a»).
//!
//! The store works on the **wire** form: a section is a [`ChunkSection`]
//! whose block container holds numeric state ids, never names. Reading a
//! block unpacks one cell; writing one rebuilds exactly one chunk — the one
//! the cell sits in — and publishes a new snapshot in which every other
//! chunk is the very same `Arc`. No lock is ever held across an `await`
//! (there is none here) or across a payload write.
//!
//! SOURCE: the wire framing this decodes and re-encodes is the recorded
//! `level_chunk_with_light` layout of `crates/protocol/src/packets/
//! chunk_payload.rs`; the section slots (Y −4..19, 24 sections) are the
//! owner's template, the same numbers `world::wire_section::SLOT` uses.

use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use data::BlockStates;
use protocol::{ChunkSection, PaletteContainer, BLOCK_CELLS};

use crate::chunks::ReadyChunks;

/// Lowest section Y a chunk payload holds.
///
/// SOURCE: `world::wire_section::SLOT` — the owner's template fills the 24
/// slots Y −4 .. 19, so the payload's `sections[0]` is section Y −4. `net`
/// cannot reach `world` (crate layout), so the number is repeated here with
/// the reference instead of derived.
pub const MIN_SECTION_Y: i32 = -4;

/// Bits per cell a block container never drops below.
///
/// SOURCE: the recorded sections use the same floor for blocks
/// (`world::wire_section::container` passes `min_bits = 4`); biome
/// containers use 1.
const BLOCK_MIN_BITS: usize = 4;

/// Highest bits per cell a palette container may declare (the codec's limit).
const MAX_BITS: usize = 16;

/// Bits of a block cell's coordinate inside one section (16 per axis).
const CELL_MASK: i32 = 15;

/// What a block read or write can fail with. Every variant names the data
/// that did not match rather than guessing a value.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StoreError {
    /// The chunk that owns the coordinate is not in the store at all.
    #[error("chunk ({x}, {z}) is not in the store")]
    ChunkMissing {
        /// Chunk x coordinate.
        x: i32,
        /// Chunk z coordinate.
        z: i32,
    },
    /// The block y is outside the section slots the dimension has.
    #[error("y={y} is outside the store's section slots")]
    OutsideWorld {
        /// Block y coordinate.
        y: i32,
    },
    /// The chunk is in the store but carries no section for that y.
    #[error("chunk ({x}, {z}) has no section for y={y}")]
    SectionMissing {
        /// Chunk x coordinate.
        x: i32,
        /// Chunk z coordinate.
        z: i32,
        /// Block y coordinate.
        y: i32,
    },
    /// The packed cells of a section do not decode under their own bit width.
    #[error("the packed cells of section y={y} are malformed")]
    Malformed {
        /// Block y coordinate (whose section did not decode).
        y: i32,
    },
    /// Extending the palette would need more bits than the codec allows.
    #[error("the section palette at y={y} would need more than 16 bits per cell")]
    PaletteTooLarge {
        /// Block y coordinate (whose section could not grow).
        y: i32,
    },
}

/// The shared world state: the current wire snapshot and the block report.
///
/// See the module docs and `docs/DECISIONS.md` («Шаг 13a») for the single
/// writer (the game thread), the many readers (connections at join and on a
/// window move) and why one `RwLock<Arc<…>>` is the whole scheme.
pub struct ChunkStore {
    /// The current immutable snapshot. Readers clone the `Arc`; the writer
    /// builds a new snapshot and swaps this pointer — the lock is held for
    /// the swap, never for a payload write.
    current: RwLock<Arc<ReadyChunks>>,
    /// The block report: state id → air/fluid, for the section headers.
    blocks: Arc<BlockStates>,
}

impl std::fmt::Debug for ChunkStore {
    /// Prints the counts, never the payloads: a `ChunkStore` sits inside
    /// [`GameConfig`](crate::game::GameConfig), whose derived `Debug` must
    /// stay cheap and bounded.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let snapshot = self.snapshot();
        f.debug_struct("ChunkStore")
            .field("chunks", &snapshot.len())
            .field("blocks", &self.blocks.len())
            .finish()
    }
}

impl ChunkStore {
    /// Wraps a ready snapshot with the block report its writes classify
    /// against.
    pub fn new(ready: ReadyChunks, blocks: Arc<BlockStates>) -> Self {
        Self {
            current: RwLock::new(Arc::new(ready)),
            blocks,
        }
    }

    /// The current snapshot, behind the shared `Arc`.
    ///
    /// A join (and every sub-batch) takes one of these; the pointer is all
    /// that is copied, never a payload. The read lock is held only for the
    /// clone itself.
    pub fn snapshot(&self) -> Arc<ReadyChunks> {
        let guard = self.read();
        Arc::clone(&guard)
    }

    /// How many chunks the store holds.
    pub fn len(&self) -> usize {
        self.snapshot().len()
    }

    /// Whether the store holds no chunk at all.
    pub fn is_empty(&self) -> bool {
        self.snapshot().is_empty()
    }

    /// The position of the chunk at `(chunk_x, chunk_z)` in the current
    /// snapshot, or `None` when the store does not hold it.
    pub fn position(&self, chunk_x: i32, chunk_z: i32) -> Option<usize> {
        self.snapshot().position(chunk_x, chunk_z)
    }

    /// The block report the store classifies with.
    pub fn blocks(&self) -> &BlockStates {
        &self.blocks
    }

    /// Whether a state id is one of the three air blocks (the same rule the
    /// section headers use).
    pub fn is_air(&self, state_id: i32) -> bool {
        self.blocks.is_air(state_id)
    }

    /// Whether a state id is water (`minecraft:water`, any level).
    ///
    /// The swimming check (Шаг 14d) goes through this: the store is the only
    /// thing in the tick that can turn a wire state id into a block name.
    pub fn is_water(&self, state_id: i32) -> bool {
        self.blocks.is_water(state_id)
    }

    /// Whether the block at `(x, y, z)` is water; a missing chunk/section is
    /// "not water" (never an error — an unloaded cell cannot submerge anyone).
    ///
    /// The caller picks `y`; the swimming rule samples the block the player's
    /// feet start in (`floor(y)`, the bounding-box bottom block — an
    /// approximation flagged in `docs/SWIMMING.md` §2).
    pub fn is_water_at(&self, x: i32, y: i32, z: i32) -> bool {
        self.block(x, y, z)
            .is_some_and(|state| self.is_water(state))
    }

    /// The state id of the block at `(x, y, z)`, or `None` when the store
    /// holds no chunk or no section for it.
    pub fn block(&self, x: i32, y: i32, z: i32) -> Option<i32> {
        let snapshot = self.snapshot();
        let position = snapshot.position(x >> 4, z >> 4)?;
        let chunk = snapshot.chunks().get(position)?;
        let section = chunk.sections.get(section_index(y).ok()?)?;
        container_state(&section.blocks, cell_index(x, y, z))
    }

    /// Sets the block at `(x, y, z)` to `state_id`, rebuilding only the
    /// chunk it sits in and publishing a new snapshot.
    ///
    /// Returns the state id the cell held (the caller needs it for the
    /// `block_update` it sends); setting the state a block already has
    /// publishes nothing and returns it unchanged.
    pub fn set_block(&self, x: i32, y: i32, z: i32, state_id: i32) -> Result<i32, StoreError> {
        let (chunk_x, chunk_z) = (x >> 4, z >> 4);
        let slot = section_index(y)?;
        let cell = cell_index(x, y, z);

        let mut guard = self.write();
        let snapshot = Arc::clone(&guard);
        let missing = || StoreError::ChunkMissing {
            x: chunk_x,
            z: chunk_z,
        };
        let section_missing = || StoreError::SectionMissing {
            x: chunk_x,
            z: chunk_z,
            y,
        };
        let position = snapshot.position(chunk_x, chunk_z).ok_or_else(missing)?;
        let chunk = snapshot.chunks().get(position).ok_or_else(missing)?;
        let section = chunk.sections.get(slot).ok_or_else(section_missing)?;
        let old = container_state(&section.blocks, cell).ok_or(StoreError::Malformed { y })?;
        if old == state_id {
            return Ok(old);
        }

        // Rebuild the one chunk: clone its payload, change the one section
        // and publish a new snapshot that swaps this chunk's pointer only.
        let mut new_chunk = chunk.as_ref().clone();
        let section = new_chunk
            .sections
            .get_mut(slot)
            .ok_or_else(section_missing)?;
        set_cell(section, cell, state_id, y)?;
        adjust_counts(section, old, state_id, &self.blocks);
        *guard = Arc::new(snapshot.with_chunk(position, Arc::new(new_chunk)));
        Ok(old)
    }

    /// Locks the current snapshot for reading, recovering from a poisoned
    /// lock: the value is one `Arc` pointer, so a panicking writer never
    /// leaves it half-updated.
    fn read(&self) -> RwLockReadGuard<'_, Arc<ReadyChunks>> {
        self.current
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Locks the current snapshot for a single write, recovering from a
    /// poisoned lock for the same reason as [`Self::read`].
    fn write(&self) -> RwLockWriteGuard<'_, Arc<ReadyChunks>> {
        self.current
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The section slot a block y belongs to, as the payload indexes it.
fn section_index(y: i32) -> Result<usize, StoreError> {
    let slot = (y >> 4) - MIN_SECTION_Y;
    usize::try_from(slot).map_err(|_| StoreError::OutsideWorld { y })
}

/// The cell of one block inside its 16×16×16 section.
///
/// The packing is the wire's: `(y & 15) << 8 | (z & 15) << 4 | (x & 15)` —
/// the same index order `world::wire_section::prepare` unpacks cells in.
fn cell_index(x: i32, y: i32, z: i32) -> usize {
    (((y & CELL_MASK) << 8) | ((z & CELL_MASK) << 4) | (x & CELL_MASK)) as usize
}

/// The state id one cell of a block container holds, or `None` when the
/// container is malformed (a bit width or packed length the codec rejects).
fn container_state(container: &PaletteContainer, cell: usize) -> Option<i32> {
    match container {
        PaletteContainer::Single(value) => Some(*value),
        PaletteContainer::Packed {
            bits,
            palette,
            data,
        } => {
            let bits = usize::try_from(*bits).ok()?;
            if !(1..=MAX_BITS).contains(&bits) {
                return None;
            }
            let index = unpack_cell(data, bits, cell)?;
            palette.get(index).copied()
        }
    }
}

/// Puts `state_id` into one cell of a section, extending the palette (and
/// its bit width) when the state is not there yet.
///
/// The container only ever grows: a state removed from a cell stays in the
/// palette. That keeps the `block_update` bytes identical (they never carry
/// the section) and leaves the re-sent chunk fully correct; pruning is not
/// needed for Шаг 13a.
fn set_cell(
    section: &mut ChunkSection,
    cell: usize,
    state_id: i32,
    y: i32,
) -> Result<(), StoreError> {
    let new_container = match &section.blocks {
        PaletteContainer::Single(value) => {
            let mut cells = vec![0_usize; BLOCK_CELLS];
            cells[cell] = 1;
            PaletteContainer::Packed {
                bits: BLOCK_MIN_BITS as i32,
                palette: vec![*value, state_id],
                data: pack_cells(&cells, BLOCK_MIN_BITS),
            }
        }
        PaletteContainer::Packed {
            bits,
            palette,
            data,
        } => {
            let bits = usize::try_from(*bits).map_err(|_| StoreError::Malformed { y })?;
            if !(1..=MAX_BITS).contains(&bits) {
                return Err(StoreError::Malformed { y });
            }
            let mut cells =
                unpack_cells(data, bits, BLOCK_CELLS).ok_or(StoreError::Malformed { y })?;
            let mut palette = palette.clone();
            let index = match palette.iter().position(|&id| id == state_id) {
                Some(index) => index,
                None => {
                    palette.push(state_id);
                    palette.len() - 1
                }
            };
            let bits_out = palette_bits(palette.len());
            if bits_out > MAX_BITS {
                return Err(StoreError::PaletteTooLarge { y });
            }
            cells[cell] = index;
            PaletteContainer::Packed {
                bits: bits_out as i32,
                palette,
                data: pack_cells(&cells, bits_out),
            }
        }
    };
    section.blocks = new_container;
    Ok(())
}

/// The two count headers after one cell changed from `old` to `new`: the
/// `non_air` header loses air, the `fluid` header gains fluid, and vice
/// versa — the same classification `world::wire_section::counts` applies.
fn adjust_counts(section: &mut ChunkSection, old: i32, new: i32, blocks: &BlockStates) {
    match (blocks.is_air(old), blocks.is_air(new)) {
        (true, false) => section.non_air = section.non_air.saturating_add(1),
        (false, true) => section.non_air = section.non_air.saturating_sub(1),
        _ => {}
    }
    match (blocks.is_fluid(old), blocks.is_fluid(new)) {
        (false, true) => section.fluid = section.fluid.saturating_add(1),
        (true, false) => section.fluid = section.fluid.saturating_sub(1),
        _ => {}
    }
}

/// Bits per cell a block palette of `count` entries needs, never below the
/// block floor and never above the codec's limit.
fn palette_bits(count: usize) -> usize {
    let mut bits = 1;
    while (1_usize << bits) < count.max(1) && bits < MAX_BITS {
        bits += 1;
    }
    bits.max(BLOCK_MIN_BITS)
}

/// One cell of packed data, read as the wire packs it (big-endian 64-bit
/// words, least significant cell first).
fn unpack_cell(data: &[u8], bits: usize, cell: usize) -> Option<usize> {
    let per_word = 64 / bits;
    let word_index = cell / per_word;
    let shift = (cell % per_word) * bits;
    let start = word_index * 8;
    let word = u64::from_be_bytes(data.get(start..start + 8)?.try_into().ok()?);
    let mask = (1_u64 << bits) - 1;
    Some(((word >> shift) & mask) as usize)
}

/// Decodes every cell of packed data; `None` when the data is shorter than
/// the declared bit width and cell count imply.
fn unpack_cells(data: &[u8], bits: usize, cells: usize) -> Option<Vec<usize>> {
    (0..cells)
        .map(|cell| unpack_cell(data, bits, cell))
        .collect()
}

/// Packs cells back into big-endian words, `bits` per cell — the inverse of
/// [`unpack_cells`], byte-identical to `world::wire_section::pack`.
fn pack_cells(cells: &[usize], bits: usize) -> Vec<u8> {
    let per_word = 64 / bits;
    let mut words = vec![0_u64; cells.len().div_ceil(per_word)];
    for (index, &value) in cells.iter().enumerate() {
        words[index / per_word] |= (value as u64) << ((index % per_word) * bits);
    }
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::{HeightmapRecord, LevelChunkWithLight, LightData};

    /// The report the store's classifications read: air, stone, a two-state
    /// log and water.
    fn test_blocks() -> Arc<BlockStates> {
        let text = r#"{
          "minecraft:air": {"states": [{"id": 0, "default": true}]},
          "minecraft:stone": {"states": [{"id": 1, "default": true}]},
          "minecraft:oak_log": {"states": [
            {"id": 2, "default": true},
            {"id": 3, "default": false, "properties": {"axis": "x"}}
          ]},
          "minecraft:water": {"states": [
            {"id": 4, "default": true, "properties": {"level": "0"}}
          ]}
        }"#;
        Arc::new(BlockStates::parse(text, "test".to_owned()).expect("valid report"))
    }

    /// One empty section slot (all air, one biome).
    fn empty_section() -> ChunkSection {
        ChunkSection {
            non_air: 0,
            fluid: 0,
            blocks: PaletteContainer::Single(0),
            biomes: PaletteContainer::Single(1),
        }
    }

    /// A chunk with every one of the 24 section slots Y −4..19.
    fn test_chunk(chunk_x: i32, chunk_z: i32) -> LevelChunkWithLight {
        LevelChunkWithLight {
            chunk_x,
            chunk_z,
            heightmaps: vec![HeightmapRecord {
                id: 4,
                words: vec![0],
            }],
            sections: (0..24).map(|_| empty_section()).collect(),
            block_entities: Vec::new(),
            light: LightData {
                sky_mask: Vec::new(),
                block_mask: Vec::new(),
                empty_sky_mask: Vec::new(),
                empty_block_mask: Vec::new(),
                sky_arrays: Vec::new(),
                block_arrays: Vec::new(),
            },
        }
    }

    /// A store over `chunks` with the fixture report.
    fn store_with(chunks: Vec<LevelChunkWithLight>) -> ChunkStore {
        ChunkStore::new(ReadyChunks::new(chunks), test_blocks())
    }

    /// Packing and unpacking invert each other, and the first word pins the
    /// measured least-significant-cell-first packing.
    #[test]
    fn packed_cells_roundtrip_in_the_wire_order() {
        let cells: Vec<usize> = (0..BLOCK_CELLS).map(|index| index % 3).collect();
        let bytes = pack_cells(&cells, 4);
        assert_eq!(bytes.len(), BLOCK_CELLS / (64 / 4) * 8);
        assert_eq!(
            &bytes[0..8],
            &[0x02, 0x10, 0x21, 0x02, 0x10, 0x21, 0x02, 0x10]
        );
        assert_eq!(
            unpack_cells(&bytes, 4, BLOCK_CELLS).expect("decodable"),
            cells
        );
        assert_eq!(pack_cells(&[], 4), Vec::<u8>::new());
    }

    /// The whole point: a write is read back at the same coordinate and
    /// nowhere else.
    #[test]
    fn a_written_block_is_read_back() {
        let store = store_with(vec![test_chunk(0, 0)]);
        assert_eq!(store.block(8, 64, 8), Some(0), "air before the write");
        assert_eq!(store.set_block(8, 64, 8, 1).expect("writable"), 0);
        assert_eq!(store.block(8, 64, 8), Some(1), "stone after the write");
        assert_eq!(store.block(9, 64, 8), Some(0), "the neighbour is untouched");
        assert_eq!(store.block(8, 64, 9), Some(0), "the other neighbour too");
    }

    /// A write publishes a new snapshot whose only changed payload is the
    /// touched chunk; every other chunk is the very same pointer.
    #[test]
    fn a_write_rebuilds_only_the_touched_chunk() {
        let store = store_with(vec![test_chunk(0, 0), test_chunk(1, 0)]);
        let before = store.snapshot();
        store.set_block(8, 64, 8, 1).expect("inside chunk (0, 0)");
        let after = store.snapshot();
        assert!(
            !Arc::ptr_eq(&before, &after),
            "a new snapshot was published"
        );
        assert!(
            !Arc::ptr_eq(&before.chunks()[0], &after.chunks()[0]),
            "the touched chunk is a new payload"
        );
        assert!(
            Arc::ptr_eq(&before.chunks()[1], &after.chunks()[1]),
            "the untouched chunk is the very same payload"
        );
        assert_eq!(after.position(1, 0), Some(1), "the index is unchanged");
    }

    /// Writing the state a block already has changes nothing at all.
    #[test]
    fn writing_the_same_state_publishes_nothing() {
        let store = store_with(vec![test_chunk(0, 0)]);
        let before = store.snapshot();
        assert_eq!(store.set_block(8, 64, 8, 0).expect("air to air"), 0);
        assert!(
            Arc::ptr_eq(&before, &store.snapshot()),
            "no snapshot was published"
        );
    }

    /// The section headers follow the change: a non-air, fluid block raises
    /// both counts, taking it away lowers them again.
    #[test]
    fn the_section_headers_follow_the_change() {
        let store = store_with(vec![test_chunk(0, 0)]);
        store.set_block(0, 64, 0, 1).expect("stone");
        let snapshot = store.snapshot();
        let section = &snapshot.chunks()[0].sections[8];
        assert_eq!(section.non_air, 1, "one non-air cell");
        assert_eq!(section.fluid, 0, "stone is not fluid");
        store.set_block(0, 64, 0, 4).expect("water");
        let snapshot = store.snapshot();
        let section = &snapshot.chunks()[0].sections[8];
        assert_eq!(section.non_air, 1, "water is not air");
        assert_eq!(section.fluid, 1, "one fluid cell");
        store.set_block(0, 64, 0, 0).expect("air again");
        let snapshot = store.snapshot();
        let section = &snapshot.chunks()[0].sections[8];
        assert_eq!(section.non_air, 0, "back to air");
        assert_eq!(section.fluid, 0, "and dry");
    }

    /// The first write into a single-valued section turns it into a packed
    /// one; a third distinct state joins the palette without disturbing the
    /// first two.
    #[test]
    fn the_palette_grows_from_single_to_packed() {
        let store = store_with(vec![test_chunk(0, 0)]);
        store.set_block(0, 64, 0, 1).expect("stone");
        store.set_block(1, 64, 0, 2).expect("log");
        let snapshot = store.snapshot();
        let section = &snapshot.chunks()[0].sections[8];
        match &section.blocks {
            PaletteContainer::Packed { palette, .. } => {
                assert_eq!(palette, &vec![0, 1, 2], "air, stone, log");
            }
            other => panic!("expected a packed container, got {other:?}"),
        }
        assert_eq!(store.block(0, 64, 0), Some(1), "stone kept");
        assert_eq!(store.block(1, 64, 0), Some(2), "log placed");
        assert_eq!(store.block(2, 64, 0), Some(0), "air still air");
    }

    /// A read or write outside the world, or one for a missing chunk, is a
    /// named error rather than a guessed value.
    #[test]
    fn out_of_world_and_missing_chunks_are_named() {
        let store = store_with(vec![test_chunk(0, 0)]);
        assert_eq!(store.block(8, 400, 8), None, "y is above every slot");
        assert_eq!(store.block(8, -100, 8), None, "y is below every slot");
        assert_eq!(store.block(999, 64, 8), None, "the chunk is not stored");
        assert!(matches!(
            store.set_block(8, 400, 8, 1),
            Err(StoreError::SectionMissing { y: 400, .. })
        ));
        assert!(matches!(
            store.set_block(8, -100, 8, 1),
            Err(StoreError::OutsideWorld { y: -100 })
        ));
        assert!(matches!(
            store.set_block(999, 64, 8, 1),
            Err(StoreError::ChunkMissing { x: 62, z: 0 })
        ));
    }

    /// A packed section decodes its cells: reading a cell the packed data
    /// placed there gives the palette entry it indexes.
    #[test]
    fn a_packed_section_reads_its_own_cells() {
        let mut chunk = test_chunk(0, 0);
        let mut cells = vec![0_usize; BLOCK_CELLS];
        cells[cell_index(3, 64, 5)] = 1;
        chunk.sections[8].blocks = PaletteContainer::Packed {
            bits: 4,
            palette: vec![0, 1],
            data: pack_cells(&cells, 4),
        };
        chunk.sections[8].non_air = 1;
        let store = store_with(vec![chunk]);
        assert_eq!(store.block(3, 64, 5), Some(1), "the packed cell");
        assert_eq!(store.block(4, 64, 5), Some(0), "a neighbouring air cell");
        store.set_block(4, 64, 5, 1).expect("place stone beside it");
        assert_eq!(store.block(3, 64, 5), Some(1), "the old cell is kept");
        assert_eq!(store.block(4, 64, 5), Some(1), "the new one landed");
        assert_eq!(store.snapshot().chunks()[0].sections[8].non_air, 2);
    }

    /// The number of chunks the store holds, and the air classification of
    /// a state id, agree with the snapshot and the report.
    #[test]
    fn the_store_exposes_its_size_and_air_rule() {
        let store = store_with(vec![test_chunk(0, 0), test_chunk(2, -2)]);
        assert_eq!(store.len(), 2);
        assert!(!store.is_empty());
        assert_eq!(store.position(2, -2), Some(1));
        assert!(store.is_air(0), "air");
        assert!(!store.is_air(1), "stone");
        assert_eq!(store.blocks().block_name(1), Some("minecraft:stone"));
    }
}
