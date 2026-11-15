//! Server-side block breaking (Шаг 13b).
//!
//! The server **trusts the client's** `player_action`: a finish is applied
//! without a server-side mining timer or hardness check. That is a
//! simplification, not the vanilla model (`docs/BLOCK-BREAKING.md` §4.2), and
//! it is marked here rather than hidden.
//!
//! What the client sees back is what the S3 trace records
//! (`docs/BLOCK-BREAKING.md` §1.2): every action is acknowledged with
//! `block_changed_ack` echoing its sequence; a finish that removes a block
//! sends `block_update` (the removed state, air) to every player — the digger
//! included — and only then the ack, exactly the order of the trace.
//!
//! SOURCE: our vanilla S3 trace (`E:\MineRust-data\autotests\2026-11-14_1644\
//! mcproxy\proxy-conn-6.jsonl`), the packet layout in `protocol` and the store
//! in `net::store`. Where the trace carries no denial case, the response is a
//! **hypothesis** and says so (see [`finish`]).

use std::sync::Arc;

use protocol::{PlayerAction, Position};
use tracing::warn;

use crate::store::ChunkStore;

use super::messages::{BlockAction, PlayerId, ServerMessage};
use super::replay::Replay;
use super::scheduler::Scheduler;

/// State id a broken block is set to.
///
/// SOURCE: the S3 trace's `block_update` carries state varint `0`, and
/// `reports/blocks.json` gives `minecraft:air` the id `0`. The store's own
/// `is_air` classification agrees with the same report.
pub const AIR_STATE: i32 = 0;

/// Applies one `player_action` (41) for `id`.
///
/// The action is recorded on the replay journal like every other applied
/// message. A connection with no world store (`store == None`, e.g. a test
/// scheduler) is only acknowledged — nothing can change without a store.
pub(super) fn apply<W: std::io::Write>(
    scheduler: &mut Scheduler,
    replay: &mut Replay<W>,
    tick: u64,
    store: Option<&Arc<ChunkStore>>,
    action: BlockAction,
) {
    let BlockAction {
        id,
        action,
        position,
        sequence,
    } = action;
    let region_id = scheduler.region_of(id);
    replay.record(
        tick,
        region_id,
        id,
        "block_action",
        serde_json::json!({
            "action": action,
            "x": position.x,
            "y": position.y,
            "z": position.z,
            "sequence": sequence,
        }),
    );
    let Some(store) = store else {
        ack(scheduler, id, sequence);
        return;
    };
    match action {
        PlayerAction::ACTION_FINISH => finish(scheduler, store, id, position, sequence),
        // Start and abort change nothing; the trace acknowledges both (start
        // carries `block_changed_ack` seq `01`, and a tap leaves the block).
        _ => ack(scheduler, id, sequence),
    }
}

/// A finished break: removes the block if it can, then acknowledges.
///
/// The success path follows the trace: `block_update` to every player (the
/// digger included), then `block_changed_ack` to the digger.
///
/// The refusal path is a **hypothesis** — the S3 trace never shows one. When
/// the store knows the block (the chunk is loaded) it is sent back so the
/// digger's prediction is corrected; when the chunk is not loaded there is no
/// state to send, so the action is only acknowledged.
fn finish(
    scheduler: &mut Scheduler,
    store: &Arc<ChunkStore>,
    id: PlayerId,
    position: Position,
    sequence: i32,
) {
    match store.block(position.x, position.y, position.z) {
        Some(state) if !store.is_air(state) => {
            match store.set_block(position.x, position.y, position.z, AIR_STATE) {
                Ok(_) => {
                    broadcast(scheduler, position, AIR_STATE);
                    ack(scheduler, id, sequence);
                }
                Err(err) => {
                    // The read above already proved the cell is writable; a
                    // failure here is a race or malformed data, never a guess.
                    warn!(
                        player = id,
                        x = position.x,
                        y = position.y,
                        z = position.z,
                        %err,
                        "block write failed, the action is only acknowledged"
                    );
                    ack(scheduler, id, sequence);
                }
            }
        }
        Some(state) => {
            // Already air: nothing changes; the digger is told the real state
            // (a hypothesis, see the doc comment).
            deliver(
                scheduler,
                id,
                ServerMessage::BlockUpdate { position, state },
            );
            ack(scheduler, id, sequence);
        }
        None => {
            // The chunk is not loaded (or y is outside the slots): no state
            // exists to send back, so only the ack goes out.
            ack(scheduler, id, sequence);
        }
    }
}

/// Sends one `block_update` to every player of the scheduler, ascending id.
///
/// M4 keeps one region (the whole world is [`SINGLE_REGION`](super::SINGLE_REGION)),
/// so "every player" is the traced audience; the per-chunk view rule is a
/// later refinement (`docs/BLOCK-BREAKING.md` §4.1).
fn broadcast(scheduler: &mut Scheduler, position: Position, state: i32) {
    let ids: Vec<PlayerId> = scheduler
        .regions()
        .iter()
        .flat_map(|region| region.player_ids())
        .collect();
    for id in ids {
        deliver(
            scheduler,
            id,
            ServerMessage::BlockUpdate { position, state },
        );
    }
}

/// Sends one `block_changed_ack` to the digger.
fn ack(scheduler: &mut Scheduler, id: PlayerId, sequence: i32) {
    deliver(scheduler, id, ServerMessage::BlockChangedAck { sequence });
}

/// Delivers one message with the shared tick policy, warning on a kick.
fn deliver(scheduler: &mut Scheduler, id: PlayerId, message: ServerMessage) {
    if let Some(kick) = scheduler.deliver(id, message) {
        warn!(
            player = kick.id,
            reason = kick.reason.message(),
            "kicking a slow player"
        );
    }
}
