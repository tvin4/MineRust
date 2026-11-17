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
//!   whole three-column).
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

/// Removes `position` and every block its break cascades into, replacing each
/// with what the S14 trace shows, and announces them.
///
/// The aimed block is announced first, then the cascade parts (the trace order:
/// tall_seagrass upper then lower, kelp top-down). A write that fails is warned
/// about and skipped: the read before it proved the cell writable, so a failure
/// is a race or malformed data, never a guess.
pub(super) fn break_blocks(
    scheduler: &mut Scheduler,
    store: &Arc<ChunkStore>,
    digger: PlayerId,
    position: Position,
    state: i32,
) {
    let mut targets = vec![(position, state, true)];
    targets.extend(
        cascade(store, position, state)
            .into_iter()
            .map(|(p, s)| (p, s, false)),
    );
    for (target, broken, aimed) in targets {
        let replacement = replacement(store, broken);
        match store.set_block(target.x, target.y, target.z, replacement) {
            Ok(_) => announce(scheduler, digger, target, broken, replacement, aimed),
            Err(err) => warn!(
                player = digger,
                x = target.x,
                y = target.y,
                z = target.z,
                %err,
                "block write failed, the cell is left unchanged"
            ),
        }
    }
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

/// The extra blocks a break removes, from the S14 vanilla trace: the other
/// half of a `tall_seagrass` and every kelp block above a `kelp`/
/// `kelp_plant`. Everything else returns no extra block.
fn cascade(store: &ChunkStore, position: Position, state: i32) -> Vec<(Position, i32)> {
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
        "minecraft:kelp" | "minecraft:kelp_plant" => {
            let mut extra = Vec::new();
            let mut y = position.y + 1;
            while let Some(above) = store.block(position.x, y, position.z) {
                if matches!(
                    store.blocks().block_name(above),
                    Some("minecraft:kelp" | "minecraft:kelp_plant")
                ) {
                    extra.push((
                        Position {
                            x: position.x,
                            y,
                            z: position.z,
                        },
                        above,
                    ));
                    y += 1;
                } else {
                    break;
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
    digger: PlayerId,
    position: Position,
    broken_state: i32,
    replacement: i32,
    aimed: bool,
) {
    let ids: Vec<PlayerId> = scheduler
        .regions()
        .iter()
        .flat_map(|region| region.player_ids())
        .collect();
    for id in &ids {
        if aimed && *id == digger {
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
