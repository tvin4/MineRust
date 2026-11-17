//! What a broken block leaves behind (Шаг 18).
//!
//! The S14 vanilla trace (`docs/BLOCK-BREAKING.md` §1.12) shows that a break
//! does not always leave air, and does not always remove one block:
//!
//! - a block that carries water — a `waterlogged=true` state or one of the
//!   water plants (`seagrass`, `tall_seagrass`, `kelp`, `kelp_plant`) — is
//!   replaced by a `minecraft:water` source (`state 89`,
//!   `reports/blocks.json`); short grass, leaf litter and a dandelion leave
//!   air;
//! - `minecraft:tall_seagrass` is a double plant: breaking either half
//!   removes the other one too (the trace carries a `2001` break effect for
//!   both halves);
//! - `minecraft:kelp`/`minecraft:kelp_plant` grow as a column: breaking a
//!   part removes every kelp block above it (the trace carries `2001` for the
//!   whole three-column). Unlike the double plant, the column above does not
//!   go at once: each block above loses its support only after the one below
//!   was removed, so one block breaks per tick (Шаг 22, owner's live check of
//!   the vanilla client). A broken kelp therefore queues the block above for
//!   the next tick instead of deleting the whole column in the break tick.
//!
//! Everything else is a single block that leaves air. The other double plants
//! (tall grass, large fern, the flowers) and the other growing plants (cave
//! vines, twisting/weeping vines) are **not** covered and are recorded as debt
//! (`docs/BLOCK-BREAKING.md` §4).
//!
//! The water classification reuses [`data::BlockStates::water_surface`], the
//! same rule the section fluid header and the feet-in-water check read.
//!
//! SOURCE: the 2026-11-17 S14 vanilla trace
//! (`E:\MineRust-data\autotests\2026-11-17_s14-vanilla`).

use std::sync::Arc;

use protocol::Position;
use tracing::warn;

use crate::store::ChunkStore;

use super::block_action::{deliver, AIR_STATE};
use super::messages::{PlayerId, ServerMessage};
use super::scheduler::Scheduler;

/// Block name of the water source a water-carrying break leaves behind.
const WATER_BLOCK: &str = "minecraft:water";

/// One kelp block above a broken one, waiting for its own tick.
///
/// Vanilla does not remove the whole column in the break tick: the block above
/// loses support only after the block below it was removed, so the column goes
/// down one block per tick. The queue keeps the store the break happened in,
/// so a tick with several columns is applied without a global lookup.
#[derive(Debug)]
pub(crate) struct PendingBreak {
    /// Server tick the block breaks on.
    pub(crate) due: u64,
    /// Cell to break.
    pub(crate) position: Position,
    /// Store holding the cell.
    pub(crate) store: Arc<ChunkStore>,
}

/// Removes `position` and every block its break cascades into, replacing each
/// with what the S14 trace shows, and announces them.
///
/// The aimed block goes first, then the instant cascade parts (the other half
/// of a `tall_seagrass`). Kelp above the aimed block is *not* removed here:
/// the block directly above is queued for the next tick, and each block then
/// queues the one above it (see [`tick_pending`]). A write that fails is warned
/// about and skipped: the read before it proved the cell writable, so a failure
/// is a race or malformed data, never a guess.
pub(super) fn break_blocks(
    scheduler: &mut Scheduler,
    store: &Arc<ChunkStore>,
    digger: PlayerId,
    position: Position,
    state: i32,
) {
    let next_due = scheduler.current_tick() + 2;
    break_one(scheduler, store, position, state, Some(digger), next_due);
    for (target, broken) in instant_cascade(store, position, state) {
        break_one(scheduler, store, target, broken, None, next_due);
    }
}

/// Applies the queued kelp blocks whose tick arrived.
///
/// Each entry re-reads its cell: a block that is no longer kelp (another break
/// or a chunk unload won the race) is skipped. A broken block queues the one
/// above it for the *next* tick, which is what makes the column fall one block
/// per tick instead of at once.
pub(super) fn tick_pending(scheduler: &mut Scheduler, tick: u64) {
    for pending in scheduler.take_due_pending_breaks(tick) {
        let PendingBreak {
            position, store, ..
        } = pending;
        let Some(state) = store.block(position.x, position.y, position.z) else {
            continue;
        };
        if !is_kelp(&store, state) {
            continue;
        }
        break_one(scheduler, &store, position, state, None, tick + 1);
    }
}

/// Removes one block, announces it and, when it was kelp, queues the block
/// above for `next_due`.
fn break_one(
    scheduler: &mut Scheduler,
    store: &Arc<ChunkStore>,
    position: Position,
    state: i32,
    aimed: Option<PlayerId>,
    next_due: u64,
) {
    let new_state = replacement(store, state);
    match store.set_block(position.x, position.y, position.z, new_state) {
        Ok(_) => {
            announce(scheduler, aimed, position, state, new_state);
            if is_kelp(store, state) {
                queue_above(scheduler, store, position, next_due);
            }
        }
        Err(err) => warn!(
            x = position.x,
            y = position.y,
            z = position.z,
            %err,
            "block write failed, the cell is left unchanged"
        ),
    }
}

/// Whether `state` is one of the two kelp blocks.
fn is_kelp(store: &ChunkStore, state: i32) -> bool {
    matches!(
        store.blocks().block_name(state),
        Some("minecraft:kelp" | "minecraft:kelp_plant")
    )
}

/// Queues the block directly above a broken kelp for `due`, when it is kelp.
fn queue_above(scheduler: &mut Scheduler, store: &Arc<ChunkStore>, position: Position, due: u64) {
    let above = Position {
        x: position.x,
        y: position.y + 1,
        z: position.z,
    };
    let Some(state) = store.block(above.x, above.y, above.z) else {
        return;
    };
    if !is_kelp(store, state) {
        return;
    }
    scheduler.push_pending_break(PendingBreak {
        due,
        position: above,
        store: Arc::clone(store),
    });
}

/// The state a broken block is replaced by: a water source when the block
/// carried water, otherwise air.
fn replacement(store: &ChunkStore, state: i32) -> i32 {
    if store.blocks().water_surface(state).is_some() {
        store.blocks().default_id(WATER_BLOCK).unwrap_or(AIR_STATE)
    } else {
        AIR_STATE
    }
}

/// The extra blocks a break removes in the same tick, from the S14 vanilla
/// trace: the other half of a `tall_seagrass`. Kelp is handled by the
/// per-tick queue instead, so it returns no extra block here.
fn instant_cascade(store: &ChunkStore, position: Position, state: i32) -> Vec<(Position, i32)> {
    let Some(name) = store.blocks().block_name(state) else {
        return Vec::new();
    };
    match name {
        "minecraft:tall_seagrass" => {
            let mut extra = Vec::new();
            for dy in [-1, 1] {
                let other = Position {
                    x: position.x,
                    y: position.y + dy,
                    z: position.z,
                };
                if let Some(other_state) = store.block(other.x, other.y, other.z) {
                    if store.blocks().block_name(other_state) == Some("minecraft:tall_seagrass") {
                        extra.push((other, other_state));
                    }
                }
            }
            extra
        }
        _ => Vec::new(),
    }
}

/// Announces one broken block: the break `level_event` (with the removed
/// state id) and then the `block_update` of the replacement.
///
/// The audience of the `level_event` depends on what the block is (S14 trace,
/// `docs/BLOCK-BREAKING.md` §1.12): the **aimed** block reaches the observers
/// only — never the digger, as in S3/S9 — while a **cascade** part (the other
/// `tall_seagrass` half, the kelp above) reaches **everyone, the digger
/// included**: the trace's digger received `2001` for the tall-seagrass lower
/// half and for kelp y203 but not for the block it aimed at. The
/// `block_update` always reaches everyone.
fn announce(
    scheduler: &mut Scheduler,
    aimed: Option<PlayerId>,
    position: Position,
    broken_state: i32,
    replacement: i32,
) {
    let ids: Vec<PlayerId> = scheduler
        .regions()
        .iter()
        .flat_map(|region| region.player_ids())
        .collect();
    for id in &ids {
        if aimed == Some(*id) {
            continue;
        }
        deliver(
            scheduler,
            *id,
            ServerMessage::LevelEvent {
                position,
                state: broken_state,
            },
        );
    }
    for id in ids {
        deliver(
            scheduler,
            id,
            ServerMessage::BlockUpdate {
                position,
                state: replacement,
            },
        );
    }
}
