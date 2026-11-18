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

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use data::{BlockStates, CollisionShapes};
use protocol::{ChunkSection, LevelChunkWithLight, PaletteContainer, BLOCK_CELLS};

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

/// A chunk whose cells differ from the template, with the generation the
/// cells reached at the moment they were changed.
#[derive(Debug)]
struct DirtyChunk {
    /// Incremented by every write to the chunk; a save acknowledgement only
    /// clears the entry when it still carries the generation it saved.
    generation: u64,
    /// Generation currently queued for the writer. `None` means the next
    /// autosave may select this chunk.
    in_flight: Option<u64>,
    /// The chunk as the template had it. `Arc` keeps one immutable comparison
    /// baseline without copying a payload.
    baseline: Arc<LevelChunkWithLight>,
    /// Changed cells relative to `baseline`: cell index → state id.
    cells: BTreeMap<u32, i32>,
}

/// A diff that was already present on disk when the world was opened.
///
/// It is not dirty by itself. Keeping its template payload and cells here
/// lets the first later edit start from the complete saved diff instead of
/// rewriting or forgetting the other saved cells.
#[derive(Debug)]
struct LoadedChunk {
    /// The template chunk the saved cells differ from.
    baseline: Arc<LevelChunkWithLight>,
    /// Cells that were read from the existing `.mrchunk` file.
    cells: BTreeMap<u32, i32>,
}

/// Every chunk with unsaved changes, in coordinate order.
#[derive(Debug, Default)]
struct DirtyState {
    /// Generation handed to the next newly dirty chunk.
    next: u64,
    /// Dirty chunks by coordinate.
    chunks: BTreeMap<(i32, i32), DirtyChunk>,
    /// The diffs each chunk was loaded with, already durable on disk.
    ///
    /// [`ChunkStore::reset_dirty`] (the development geometry) clears new
    /// changes but keeps this state, so a loaded diff is neither written back
    /// by a development-only edit nor lost when a real edit happens later.
    loaded: BTreeMap<(i32, i32), LoadedChunk>,
}

/// One chunk handed to the world-save writer: the cells that differ from the
/// template and the generation they belong to.
///
/// The tick builds these by copying the per-chunk change map and hands them to
/// the writer thread; nothing here holds the store lock after the snapshot
/// returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkSave {
    /// Chunk x coordinate.
    pub chunk_x: i32,
    /// Chunk z coordinate.
    pub chunk_z: i32,
    /// Generation of the chunk when the snapshot was taken.
    pub generation: u64,
    /// Changed cells, sorted: `(cell index, state id)`.
    pub changes: Vec<(u32, i32)>,
}

/// What the writer reports after one chunk: whether its file now matches the
/// snapshot of `generation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkSaved {
    /// Chunk x coordinate.
    pub chunk_x: i32,
    /// Chunk z coordinate.
    pub chunk_z: i32,
    /// Generation the writer saved.
    pub generation: u64,
    /// Whether the file (or its deletion) was written successfully.
    pub ok: bool,
}

/// A chunk read back from the save: the cells that differ from the template,
/// with the template payload they were compared against (Шаг 22b).
#[derive(Debug, Clone)]
pub struct SavedChunk {
    /// Chunk x coordinate.
    pub chunk_x: i32,
    /// Chunk z coordinate.
    pub chunk_z: i32,
    /// The template chunk, before the saved cells were applied.
    pub baseline: Arc<LevelChunkWithLight>,
    /// Saved cells, sorted: `(cell index, state id)`.
    pub cells: Vec<(u32, i32)>,
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
    /// The collision report: state id → collision-shape bottom, for the
    /// free-headroom query of the crawl rule (Шаг 15 п.5). Empty when the
    /// server was started without `collision_report`.
    collisions: Arc<CollisionShapes>,
    /// Cells changed against the template, per chunk (Шаг 22b).
    ///
    /// The lock is taken only by a block write and by the autosave snapshot,
    /// never by a tick that changes no block (docs/DECISIONS.md «Шаг 22b»).
    dirty: Mutex<DirtyState>,
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
            .field("collisions", &self.collisions.len())
            .finish()
    }
}

impl ChunkStore {
    /// Wraps a ready snapshot with the block report its writes classify
    /// against.
    pub fn new(ready: ReadyChunks, blocks: Arc<BlockStates>) -> Self {
        Self::with_collisions(ready, blocks, Arc::new(CollisionShapes::default()))
    }

    /// Like [`Self::new`], with the collision report the free-headroom query
    /// reads. Without it [`Self::free_headroom`] answers `None` and the forced
    /// crouch/crawl rule stays off.
    pub fn with_collisions(
        ready: ReadyChunks,
        blocks: Arc<BlockStates>,
        collisions: Arc<CollisionShapes>,
    ) -> Self {
        Self {
            current: RwLock::new(Arc::new(ready)),
            blocks,
            collisions,
            dirty: Mutex::new(DirtyState::default()),
        }
    }

    /// Like [`Self::with_collisions`], plus the chunks a previous run saved:
    /// their cells are already applied to `ready`, and the template payloads
    /// in `saved` become the baseline a later write is compared against, so a
    /// cell set back to its template value leaves the diff (Шаг 22b).
    pub fn with_saved(
        ready: ReadyChunks,
        blocks: Arc<BlockStates>,
        collisions: Arc<CollisionShapes>,
        saved: Vec<SavedChunk>,
    ) -> Self {
        let store = Self::with_collisions(ready, blocks, collisions);
        if !saved.is_empty() {
            let mut dirty = store.lock_dirty();
            for chunk in saved {
                dirty.loaded.insert(
                    (chunk.chunk_x, chunk.chunk_z),
                    LoadedChunk {
                        baseline: chunk.baseline,
                        cells: chunk.cells.into_iter().collect(),
                    },
                );
            }
        }
        store
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
    /// This is the block-name check of Шаг 14d. The Шаг 14e swimming rule no
    /// longer uses it: it reads the fluid surface through
    /// [`Self::feet_in_water`] so a waterlogged block the player stands on top
    /// of does not count.
    pub fn is_water_at(&self, x: i32, y: i32, z: i32) -> bool {
        self.block(x, y, z)
            .is_some_and(|state| self.is_water(state))
    }

    /// Height of the water surface above the block base at `(x, y, z)`, or
    /// `None` when the cell holds no water (or the chunk is not loaded).
    pub fn water_surface_at(&self, x: i32, y: i32, z: i32) -> Option<f32> {
        self.block(x, y, z)
            .and_then(|state| self.blocks.water_surface(state))
    }

    /// Whether a player whose feet are at the continuous height `feet_y` is in
    /// water at column `(x, z)`.
    ///
    /// The Шаг 14e rule: the feet block must carry water whose surface is
    /// *above* the feet, so a player standing on top of a source, a top slab
    /// or a stair is dry, while one standing inside a source, a waterlogged
    /// block or a water plant is wet. This replaces the Шаг 14d "the feet
    /// block is `minecraft:water`" check (`docs/SWIMMING.md` §4).
    pub fn feet_in_water(&self, x: i32, feet_y: f64, z: i32) -> bool {
        self.point_in_water(x, feet_y, z)
    }

    /// Whether the point at continuous height `y` in column `(x, z)` is
    /// inside water — the same surface test [`Self::feet_in_water`] applies
    /// at the feet.
    ///
    /// The swimming start gate reads this at the eye height
    /// (`y + STANDING_EYE_HEIGHT`); a missing block or a dry surface is
    /// `false`, never an error.
    ///
    /// A water block with water directly above it is one continuous column:
    /// the source's own `8/9` surface is only the *visible* top when air is
    /// above, so the top `1/9` of a submerged cell is water too. Without this
    /// the feet (or eyes) crossed a fake `1/9` gap at every block boundary and
    /// the swim bit toggled on each rising block — the surface-swim flicker of
    /// the 2026-11-16 live test (see the store unit test). SOURCE: the live
    /// session trace `E:\MineRust-data\live-repro-p2\session.json`, where the
    /// swim flag toggled (`24 08`) exactly as the feet's fractional height
    /// crossed `8/9` inside a full water column.
    pub fn point_in_water(&self, x: i32, y: f64, z: i32) -> bool {
        let block_y = y.floor() as i32;
        let Some(surface) = self.water_surface_at(x, block_y, z) else {
            return false;
        };
        if (y - f64::from(block_y)) < f64::from(surface) {
            return true;
        }
        // In the source's own 8/9 "shadow": still water when the column
        // continues into the cell above.
        self.is_water_at(x, block_y + 1, z)
    }

    /// Free space above the feet of a player standing at `(x, feet_y, z)`,
    /// measured to the lowest collision shape directly above it.
    ///
    /// Шаг 15 п.5: the vanilla S11 run forces the pose from this number. The
    /// player footprint is 0.6 wide, so up to two block columns may overlap per
    /// axis; every column's cell is checked from the feet cell up to two blocks
    /// above, and the lowest obstruction wins. The result saturates at `2.0`
    /// because the rule only distinguishes `< 1.5` (crawl), `1.5 .. 1.8`
    /// (crouch) and `>= 1.8` (standing).
    ///
    /// `None` when the store has no collision report: the rule cannot be
    /// evaluated and the caller keeps the pose it had.
    ///
    /// SOURCE: the vanilla S11 timeline (`docs/SWIMMING.md` §5): a full block at
    /// the head cell (feet `201.0`, block at `202.0`) gives headroom `1.0` and
    /// pose SWIMMING (crawl, measured hitbox `0.6`); a top slab in the same
    /// cell gives `1.5` and CROUCHING (`1.5`); the open cell gives `>= 1.8`
    /// and STANDING (`1.8`). The per-state bottoms come from the
    /// `reports/collision.json` the game's own `getCollisionShape` produced.
    pub fn free_headroom(&self, x: f64, feet_y: f64, z: f64) -> Option<f64> {
        if self.collisions.is_empty() {
            return None;
        }
        const HALF_WIDTH: f64 = 0.3;
        let mut columns = [(0i32, 0i32); 4];
        let mut count = 0usize;
        let xs = [x - HALF_WIDTH, x + HALF_WIDTH];
        let zs = [z - HALF_WIDTH, z + HALF_WIDTH];
        for &cx in &xs {
            for &cz in &zs {
                let cell = (cx.floor() as i32, cz.floor() as i32);
                if !columns[..count].contains(&cell) {
                    columns[count] = cell;
                    count += 1;
                }
            }
        }
        let first = feet_y.floor() as i32;
        let last = (feet_y + 2.0).floor() as i32;
        let mut headroom = 2.0_f64;
        for &(bx, bz) in &columns[..count] {
            for by in first..=last {
                let Some(state) = self.block(bx, by, bz) else {
                    continue;
                };
                let Some(bottom) = self.collisions.bottom(state) else {
                    continue;
                };
                let obstruction = by as f64 + bottom;
                if obstruction > feet_y {
                    headroom = headroom.min(obstruction - feet_y);
                }
            }
        }
        Some(headroom)
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
        self.record_change(chunk, slot, cell, old, state_id);
        Ok(old)
    }

    /// Records one changed cell of a chunk for the world save (Шаг 22b).
    ///
    /// `chunk` is the payload the write started from: on the first change of
    /// a chunk it becomes the baseline (an `Arc` clone), so the map keeps the
    /// difference against the template, not against the previous edit. A cell
    /// set back to its baseline value leaves the map, which is what turns the
    /// file into an empty diff (and its deletion).
    fn record_change(
        &self,
        chunk: &Arc<LevelChunkWithLight>,
        slot: usize,
        cell: usize,
        old: i32,
        state_id: i32,
    ) {
        let key = (chunk.chunk_x, chunk.chunk_z);
        let mut dirty = self.lock_dirty();
        let state = &mut *dirty;
        let generation = match state.chunks.get(&key) {
            Some(entry) => entry.generation.saturating_add(1),
            None => state.next,
        };
        if generation >= state.next {
            state.next = generation.saturating_add(1);
        }
        let loaded = state.loaded.get(&key);
        let baseline = loaded
            .map(|saved| Arc::clone(&saved.baseline))
            .unwrap_or_else(|| Arc::clone(chunk));
        let cells = loaded.map(|saved| saved.cells.clone()).unwrap_or_default();
        let entry = state.chunks.entry(key).or_insert_with(|| DirtyChunk {
            generation,
            in_flight: None,
            baseline,
            cells,
        });
        entry.generation = generation;
        // A new generation is eligible even while an older snapshot of this
        // chunk is still in the writer queue.
        entry.in_flight = None;
        // The baseline always has the section: the write above read that very
        // section from it. A malformed baseline falls back to `old`, which
        // keeps the cell in the diff (the safe direction).
        let template = entry
            .baseline
            .sections
            .get(slot)
            .and_then(|section| container_state(&section.blocks, cell))
            .unwrap_or(old);
        // The save addresses cells section-major: the container index is only
        // the cell inside one section (Шаг 22b).
        let cell = (slot * BLOCK_CELLS + cell) as u32;
        if state_id == template {
            entry.cells.remove(&cell);
        } else {
            entry.cells.insert(cell, state_id);
        }
    }

    /// Up to `limit` dirty chunks, in coordinate order, for the writer thread.
    ///
    /// The entries stay in the map: only a successful [`Self::saved`] with the
    /// same generation removes one, so an edit during the write keeps the
    /// chunk dirty.
    pub fn autosave_batch(&self, limit: usize) -> Vec<ChunkSave> {
        let mut dirty = self.lock_dirty();
        let mut batch = Vec::with_capacity(limit.min(dirty.chunks.len()));
        for (&(chunk_x, chunk_z), entry) in &mut dirty.chunks {
            if batch.len() == limit {
                break;
            }
            if entry.in_flight.is_some() {
                continue;
            }
            entry.in_flight = Some(entry.generation);
            batch.push(ChunkSave {
                chunk_x,
                chunk_z,
                generation: entry.generation,
                changes: entry
                    .cells
                    .iter()
                    .map(|(&cell, &state)| (cell, state))
                    .collect(),
            });
        }
        batch
    }

    /// Every dirty chunk at its newest generation, for the final flush after
    /// the game thread has stopped.
    ///
    /// Unlike [`Self::autosave_batch`], this includes snapshots already in
    /// flight: the final writer command is queued after them and overwrites
    /// each file with the newest complete diff.
    pub fn shutdown_batch(&self) -> Vec<ChunkSave> {
        let dirty = self.lock_dirty();
        dirty
            .chunks
            .iter()
            .map(|(&(chunk_x, chunk_z), entry)| ChunkSave {
                chunk_x,
                chunk_z,
                generation: entry.generation,
                changes: entry
                    .cells
                    .iter()
                    .map(|(&cell, &state)| (cell, state))
                    .collect(),
            })
            .collect()
    }

    /// Makes a batch eligible again when a nonblocking queue submission was
    /// refused. Only the same generation is cleared: a later edit has already
    /// reset its own in-flight marker.
    pub fn not_queued(&self, batch: &[ChunkSave]) {
        let mut dirty = self.lock_dirty();
        for save in batch {
            if let Some(entry) = dirty.chunks.get_mut(&(save.chunk_x, save.chunk_z)) {
                if entry.in_flight == Some(save.generation) {
                    entry.in_flight = None;
                }
            }
        }
    }

    /// Applies one writer report: a successful save of the generation the
    /// chunk still carries drops the entry, so it is not written again.
    pub fn saved(&self, done: ChunkSaved) {
        let mut dirty = self.lock_dirty();
        let key = (done.chunk_x, done.chunk_z);
        if !done.ok {
            if let Some(entry) = dirty.chunks.get_mut(&key) {
                if entry.in_flight == Some(done.generation) {
                    entry.in_flight = None;
                }
            }
            return;
        }
        if dirty
            .chunks
            .get(&key)
            .is_some_and(|entry| entry.generation == done.generation)
        {
            let Some(entry) = dirty.chunks.remove(&key) else {
                return;
            };
            if entry.cells.is_empty() {
                dirty.loaded.remove(&key);
            } else {
                dirty.loaded.insert(
                    key,
                    LoadedChunk {
                        baseline: entry.baseline,
                        cells: entry.cells,
                    },
                );
            }
        }
    }

    /// How many chunks carry unsaved changes.
    ///
    /// Diffs read at startup are already durable and do not count until a
    /// later block write changes their chunk.
    pub fn dirty_chunks(&self) -> usize {
        self.lock_dirty().chunks.len()
    }

    /// Forgets every change: the in-memory world stays as it is, but nothing
    /// is saved.
    ///
    /// This is the startup hook of the development geometry (Шаг 22b): loaded
    /// diffs already live in `loaded`, so clearing only the newly dirty map
    /// drops the geometry without scheduling an unchanged save file for a
    /// rewrite. Call it before the first autosave, never later: it would
    /// discard real edits too.
    pub fn reset_dirty(&self) {
        self.lock_dirty().chunks.clear();
    }

    /// Locks the dirty map, recovering from a poisoned lock like
    /// [`Self::read`]: the map is rebuilt from whole entries, never left
    /// half-updated by a panic.
    fn lock_dirty(&self) -> MutexGuard<'_, DirtyState> {
        self.dirty
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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

/// Number of block cells one payload holds across all its sections.
pub fn chunk_cell_count(chunk: &LevelChunkWithLight) -> u32 {
    (chunk.sections.len() * BLOCK_CELLS) as u32
}

/// The state id of one cell of a chunk payload, addressed by the flat
/// section-major index the world save stores (`Шаг 22b`).
///
/// `None` when the index is outside the payload or its container is
/// malformed — the same "no guess" rule [`ChunkStore::block`] follows.
pub fn chunk_cell_state(chunk: &LevelChunkWithLight, cell: u32) -> Option<i32> {
    let cell = cell as usize;
    let section = chunk.sections.get(cell / BLOCK_CELLS)?;
    container_state(&section.blocks, cell % BLOCK_CELLS)
}

/// Puts `state_id` into one cell of a chunk payload, extending the palette
/// and fixing the section counts — the read of a saved diff (`Шаг 22b`).
pub fn set_chunk_cell(
    chunk: &mut LevelChunkWithLight,
    cell: u32,
    state_id: i32,
    blocks: &BlockStates,
) -> Result<(), StoreError> {
    let cell = cell as usize;
    let slot = cell / BLOCK_CELLS;
    let local = cell % BLOCK_CELLS;
    let y = (MIN_SECTION_Y + slot as i32) * 16;
    let section = chunk
        .sections
        .get_mut(slot)
        .ok_or(StoreError::Malformed { y })?;
    let old = container_state(&section.blocks, local).ok_or(StoreError::Malformed { y })?;
    if old == state_id {
        return Ok(());
    }
    set_cell(section, local, state_id, y)?;
    adjust_counts(section, old, state_id, blocks);
    Ok(())
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

    /// A store over `chunks` with a collision report mapping stone (1) to a
    /// full-cell bottom and the oak log's x-axis state (3) to a top-slab-like
    /// `0.5`, so [`ChunkStore::free_headroom`] has a shape to read.
    fn store_with_collisions(chunks: Vec<LevelChunkWithLight>) -> ChunkStore {
        let report = r#"{"shapes": {"1": 0.0, "3": 0.5}}"#;
        let collisions =
            CollisionShapes::parse(report, "test".to_owned()).expect("valid collision report");
        ChunkStore::with_collisions(
            ReadyChunks::new(chunks),
            test_blocks(),
            Arc::new(collisions),
        )
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

    /// The Шаг 14e feet check reads the water *surface*, not the block name:
    /// feet inside a source are wet, feet above it are dry, and a missing cell
    /// is dry.
    #[test]
    fn the_feet_check_reads_the_water_surface() {
        let store = store_with(vec![test_chunk(0, 0)]);
        store.set_block(0, 64, 0, 4).expect("a source");
        assert!(store.feet_in_water(0, 64.0, 0), "feet at the base");
        assert!(store.feet_in_water(0, 64.5, 0), "half a block up");
        assert!(
            !store.feet_in_water(0, 64.9, 0),
            "above the 8/9 surface is dry"
        );
        assert!(!store.feet_in_water(0, 65.0, 0), "on top of the block");
        assert!(!store.feet_in_water(1, 64.0, 0), "stone holds no water");
        assert!(!store.feet_in_water(0, 200.0, 0), "no section, no water");
        // The same test at any continuous height (the swimming start gate
        // reads it at the eye): inside the source yes, above the surface no.
        assert!(store.point_in_water(0, 64.2, 0), "inside the source");
        assert!(!store.point_in_water(0, 64.95, 0), "above the surface");
        assert!(!store.point_in_water(0, 65.0, 0), "the cell above");
    }

    /// A water block with water directly above it is one continuous column:
    /// the source's own `8/9` surface is only the *visible* top when air is
    /// above, so the top `1/9` of a submerged cell is still water.
    ///
    /// This is the surface-swim flicker of 2026-11-16 (live two-client test):
    /// a player rising through a water column crossed the fake gap at every
    /// block boundary, the swim bit toggled on and off (`24 08` in the server
    /// session trace `E:\MineRust-data\live-repro-p2\session.json`) and the
    /// observer saw the swim animation reset and restart.
    #[test]
    fn a_water_block_under_water_fills_its_cell() {
        let store = store_with(vec![test_chunk(0, 0)]);
        store.set_block(0, 64, 0, 4).expect("a source");
        store.set_block(0, 65, 0, 4).expect("a source above");
        // The top 1/9 of the lower cell is water, not the 8/9 surface shadow.
        assert!(store.point_in_water(0, 64.95, 0), "inside the column");
        assert!(store.feet_in_water(0, 64.95, 0), "feet in the column");
        // The column's own top cell still tops out at 8/9 (air above it).
        assert!(store.point_in_water(0, 65.5, 0), "inside the top cell");
        assert!(!store.point_in_water(0, 65.95, 0), "above the column");
        assert!(!store.feet_in_water(0, 65.95, 0), "feet above the column");
        // A lone source with air above keeps its own 8/9 surface.
        store.set_block(0, 65, 0, 0).expect("clear the cell above");
        assert!(
            !store.point_in_water(0, 64.95, 0),
            "a lone source tops at 8/9"
        );
    }

    /// The free-headroom query (Шаг 15 п.5): the lowest collision box above the
    /// feet, measured over the player's footprint columns; `None` without a
    /// collision report, the S11 heights otherwise.
    #[test]
    fn the_free_headroom_reads_the_collision_report() {
        // Without a collision report the rule stays off: the pose is not
        // forced and the caller keeps what it had.
        let bare = store_with(vec![test_chunk(0, 0)]);
        assert_eq!(
            bare.free_headroom(8.5, 64.0, 8.5),
            None,
            "no report, no rule"
        );

        let store = store_with_collisions(vec![test_chunk(0, 0)]);
        // The open column: nothing above, so the value saturates at 2.0.
        assert_eq!(store.free_headroom(8.5, 64.0, 8.5), Some(2.0), "open");
        // A full block in the head cell (feet 64.0, block at y=65): 1.0, the
        // S11 stone value that forces a crawl.
        store.set_block(8, 65, 8, 1).expect("stone above");
        assert_eq!(store.free_headroom(8.5, 64.0, 8.5), Some(1.0), "stone");
        // A top-slab-like block (the x-axis log state): 1.5, the S11 slab
        // value that forces a crouch.
        store.set_block(8, 65, 8, 3).expect("slab-like above");
        assert_eq!(store.free_headroom(8.5, 64.0, 8.5), Some(1.5), "top slab");
        // A block whose top is not above the feet does not lower the headroom
        // (the query only counts obstruction *above* the feet).
        store.set_block(8, 65, 8, 0).expect("cleared");
        store
            .set_block(8, 64, 8, 1)
            .expect("a block at the feet cell");
        assert_eq!(
            store.free_headroom(8.5, 64.5, 8.5),
            Some(2.0),
            "a block below the feet does not obstruct the head"
        );
        // The footprint is 0.6 wide, so at x=8.9 the columns 8 and 9 both
        // count: a block one column over still lowers the headroom.
        store
            .set_block(9, 65, 8, 1)
            .expect("stone in the far column");
        store.set_block(8, 65, 8, 0).expect("cleared near column");
        assert_eq!(
            store.free_headroom(8.9, 64.0, 8.5),
            Some(1.0),
            "the far column of the footprint counts"
        );
        assert_eq!(
            store.free_headroom(8.2, 64.0, 8.5),
            Some(2.0),
            "a 0.6-wide box at x=8.2 stays inside column 8"
        );
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

    /// The flat cell index of `(x, y, z)` the save format uses: section slot
    /// first, then the wire cell inside the section.
    fn flat_cell(x: i32, y: i32, z: i32) -> u32 {
        let slot = (y >> 4) - MIN_SECTION_Y;
        let local = ((y & 15) << 8) | ((z & 15) << 4) | (x & 15);
        slot as u32 * BLOCK_CELLS as u32 + local as u32
    }

    /// A write marks exactly its chunk, the batch carries the cell, and a
    /// successful save of that generation drops the chunk (Шаг 22b).
    #[test]
    fn a_write_is_saved_once_and_then_leaves_the_dirty_set() {
        let store = store_with(vec![test_chunk(0, 0), test_chunk(1, 0)]);
        store.set_block(8, 64, 8, 1).expect("stone");
        assert_eq!(store.dirty_chunks(), 1, "only the touched chunk is dirty");

        let batch = store.autosave_batch(256);
        assert_eq!(batch.len(), 1);
        let save = &batch[0];
        assert_eq!((save.chunk_x, save.chunk_z), (0, 0));
        assert_eq!(save.changes, vec![(flat_cell(8, 64, 8), 1)]);
        let generation = save.generation;

        store.saved(ChunkSaved {
            chunk_x: 0,
            chunk_z: 0,
            generation,
            ok: true,
        });
        assert_eq!(store.dirty_chunks(), 0, "the saved chunk is clean");
        assert!(
            store.autosave_batch(256).is_empty(),
            "nothing to write twice"
        );

        store.set_block(9, 64, 9, 2).expect("a later edit");
        assert_eq!(
            store.autosave_batch(256)[0].changes,
            vec![(flat_cell(8, 64, 8), 1), (flat_cell(9, 64, 9), 2)],
            "a later save keeps the cells already durable in the same file"
        );
    }

    /// A write that lands after the snapshot keeps the chunk dirty even when
    /// the older generation is acknowledged.
    #[test]
    fn a_write_during_the_save_keeps_the_chunk_dirty() {
        let store = store_with(vec![test_chunk(0, 0)]);
        store.set_block(8, 64, 8, 1).expect("stone");
        let first = store.autosave_batch(256);
        let generation = first[0].generation;

        store.set_block(8, 64, 8, 2).expect("log");
        store.saved(ChunkSaved {
            chunk_x: 0,
            chunk_z: 0,
            generation,
            ok: true,
        });
        assert_eq!(store.dirty_chunks(), 1, "the newer edit is still pending");

        let second = store.autosave_batch(256);
        assert_eq!(second[0].changes, vec![(flat_cell(8, 64, 8), 2)]);
    }

    /// Setting a cell back to the template value leaves an empty diff: the
    /// writer deletes the file instead of keeping a stale one.
    #[test]
    fn a_cell_returned_to_the_template_leaves_an_empty_diff() {
        let store = store_with(vec![test_chunk(0, 0)]);
        store.set_block(8, 64, 8, 1).expect("stone");
        store.set_block(8, 64, 8, 0).expect("air again");

        let batch = store.autosave_batch(256);
        assert_eq!(batch.len(), 1, "the chunk still needs its file removed");
        assert!(batch[0].changes.is_empty(), "the diff is empty");
    }

    /// A failed save keeps the chunk dirty for the next autosave.
    #[test]
    fn a_failed_save_keeps_the_chunk_dirty() {
        let store = store_with(vec![test_chunk(0, 0)]);
        store.set_block(8, 64, 8, 1).expect("stone");
        let save = store.autosave_batch(256).remove(0);
        store.saved(ChunkSaved {
            chunk_x: 0,
            chunk_z: 0,
            generation: save.generation,
            ok: false,
        });
        assert_eq!(store.dirty_chunks(), 1);
        assert_eq!(
            store.autosave_batch(1).len(),
            1,
            "the negative ack makes the generation eligible for retry"
        );
    }

    /// A slow first batch is not copied into the queue repeatedly: the next
    /// pass advances to later dirty chunks, and a refused submission can make
    /// its generation eligible again.
    #[test]
    fn an_in_flight_batch_does_not_starve_the_dirty_tail() {
        let store = store_with(vec![test_chunk(0, 0), test_chunk(1, 0)]);
        store.set_block(8, 64, 8, 1).expect("first chunk");
        store.set_block(24, 64, 8, 2).expect("second chunk");

        let first = store.autosave_batch(1);
        assert_eq!((first[0].chunk_x, first[0].chunk_z), (0, 0));
        let second = store.autosave_batch(1);
        assert_eq!(
            (second[0].chunk_x, second[0].chunk_z),
            (1, 0),
            "the next pass skips the generation already in flight"
        );
        assert!(
            store.autosave_batch(1).is_empty(),
            "both dirty generations are already queued"
        );

        store.not_queued(&first);
        let retry = store.autosave_batch(1);
        assert_eq!((retry[0].chunk_x, retry[0].chunk_z), (0, 0));
    }

    /// A chunk loaded from the save keeps its template baseline: a cell set
    /// back to the template value becomes an empty diff.
    #[test]
    fn a_loaded_diff_keeps_the_template_as_its_baseline() {
        let template = test_chunk(0, 0);
        let mut loaded = template.clone();
        set_chunk_cell(&mut loaded, flat_cell(8, 64, 8), 1, &test_blocks()).expect("cell");
        let baseline = Arc::new(template);
        let store = ChunkStore::with_saved(
            ReadyChunks::new(vec![loaded]),
            test_blocks(),
            Arc::new(CollisionShapes::default()),
            vec![SavedChunk {
                chunk_x: 0,
                chunk_z: 0,
                baseline,
                cells: vec![(flat_cell(8, 64, 8), 1)],
            }],
        );
        assert_eq!(store.block(8, 64, 8), Some(1), "the saved cell is there");
        assert_eq!(
            store.dirty_chunks(),
            0,
            "a diff already durable on disk is not dirty after startup"
        );
        assert!(
            store.autosave_batch(16).is_empty(),
            "the first autosave does not rewrite an unchanged loaded diff"
        );

        store.set_block(8, 64, 8, 0).expect("back to air");
        assert!(
            store.autosave_batch(16)[0].changes.is_empty(),
            "the template value leaves no diff"
        );
    }

    /// A reset forgets development geometry without scheduling an unchanged
    /// loaded diff for another write.
    #[test]
    fn a_reset_keeps_loaded_diffs_and_drops_geometry_edits() {
        let template = test_chunk(0, 0);
        let mut loaded = template.clone();
        set_chunk_cell(&mut loaded, flat_cell(8, 64, 8), 1, &test_blocks()).expect("cell");
        let store = ChunkStore::with_saved(
            ReadyChunks::new(vec![loaded]),
            test_blocks(),
            Arc::new(CollisionShapes::default()),
            vec![SavedChunk {
                chunk_x: 0,
                chunk_z: 0,
                baseline: Arc::new(template),
                cells: vec![(flat_cell(8, 64, 8), 1)],
            }],
        );
        store.set_block(9, 64, 9, 2).expect("geometry edit");
        store.reset_dirty();
        assert_eq!(
            store.dirty_chunks(),
            0,
            "nothing changed since the disk save"
        );
        assert_eq!(
            store.block(9, 64, 9),
            Some(2),
            "the in-memory geometry stays"
        );
        assert!(
            store.autosave_batch(16).is_empty(),
            "neither the stored diff nor the geometry is queued"
        );

        store.set_block(10, 64, 10, 2).expect("real edit");
        assert_eq!(
            store.autosave_batch(16)[0].changes,
            vec![(flat_cell(8, 64, 8), 1), (flat_cell(10, 64, 10), 2)],
            "a later real edit preserves the loaded cells in the next diff"
        );
    }

    /// The chunk-level cell helpers address the same cell as the world
    /// coordinates, and an index outside the payload is an error.
    #[test]
    fn chunk_cells_are_addressed_by_the_flat_index() {
        let mut chunk = test_chunk(0, 0);
        assert_eq!(chunk_cell_count(&chunk), 24 * BLOCK_CELLS as u32);
        let cell = flat_cell(3, -64, 12);
        set_chunk_cell(&mut chunk, cell, 1, &test_blocks()).expect("inside the chunk");
        assert_eq!(chunk_cell_state(&chunk, cell), Some(1));
        assert_eq!(chunk.sections[0].non_air, 1, "the count was fixed");
        assert_eq!(chunk_cell_state(&chunk, chunk_cell_count(&chunk)), None);
    }
}
