//! Server-side block breaking (Шаг 13b, Шаг 14c, Шаг 16a.3).
//!
//! The client's `player_action` is what starts and finishes a dig. Шаг 16a.3
//! adds the survival timer on top: with the hardness report loaded, a start
//! records a [`DigState`](super::region::DigState), the tick advances it and
//! the observers see `block_destruction` stages (`super::dig`). The finish
//! stays the client's signal that removes the block — 16a.2 established that
//! the server never breaks by progress — but it must arrive plausibly on time
//! (see [`finish`]). Without `hardness_report` the timer stays off and the
//! client is trusted, exactly as before.
//!
//! What the client sees back is what the S3 trace records
//! (`docs/BLOCK-BREAKING.md` §1.2): every action is acknowledged with
//! `block_changed_ack` echoing its sequence; a finish that removes a block
//! sends a `0xFF` crack reset to the observers, then `level_event` (the
//! block-break effect, the digger excluded) and `block_update` (the removed
//! state, air) to every player, the digger included, and only then the ack.
//!
//! In creative the start action already breaks the block (Шаг 14c): the
//! vanilla S9 trace shows the digger holding attack for ~900 ms and two
//! blocks gone, with no start/finish round trip per block. The timer and the
//! hardness rules never apply to creative.
//!
//! SOURCE: our vanilla S3 trace (`E:\MineRust-data\autotests\2026-11-14_1644\
//! mcproxy\proxy-conn-6.jsonl`), the S9 creative trace
//! (`docs/BLOCK-BREAKING.md` §1.5), the stage traces of §1.7, the packet
//! layout in `protocol` and the store in `net::store`. Where the trace carries
//! no denial case, the response is a **hypothesis** and says so (see
//! [`finish`]).

use std::sync::Arc;

use protocol::{PlayerAction, Position};
use tracing::{debug, warn};

use crate::store::ChunkStore;

use super::dig::{self, DigRules, PROGRESS_DIVISOR, TOOL_REQUIRED_DIVISOR};
use super::messages::{BlockAction, PlayerId, ServerMessage};
use super::region::DigState;
use super::replay::Replay;
use super::scheduler::Scheduler;

/// State id a broken block is set to.
///
/// SOURCE: the S3 trace's `block_update` carries state varint `0`, and
/// `reports/blocks.json` gives `minecraft:air` the id `0`. The store's own
/// `is_air` classification agrees with the same report.
pub const AIR_STATE: i32 = 0;

/// Game mode id of creative.
///
/// SOURCE: `entity_packets` already encodes the creative login (the same
/// ordering across survival/creative/adventure/spectator).
pub const CREATIVE: i32 = 1;

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
    rules: DigRules<'_>,
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
    let creative = game_mode(scheduler, id) == Some(CREATIVE);
    if creative
        && matches!(
            action,
            PlayerAction::ACTION_START | PlayerAction::ACTION_FINISH
        )
    {
        // The instant break, unchanged by the timer: a start is a finish.
        finish(
            scheduler,
            store,
            DigRules::trusting(),
            id,
            position,
            sequence,
        );
        return;
    }
    match action {
        PlayerAction::ACTION_START => start(scheduler, store, rules, id, position, sequence),
        PlayerAction::ACTION_FINISH => finish(scheduler, store, rules, id, position, sequence),
        PlayerAction::ACTION_ABORT => abort(scheduler, id, sequence),
        // Unknown actions only acknowledge; the traces carry no other value.
        _ => ack(scheduler, id, sequence),
    }
}

/// Starts the survival timer when the hardness data can describe the dig.
///
/// No timer starts — the action is only acknowledged, as before — when the
/// chunk is not loaded, the cell is air, the report is absent or does not
/// describe the state, the block is unbreakable (`hardness < 0`; a finish on
/// it is refused by [`finish`]) or instant (`hardness = 0`; the finish is
/// trusted). A repeated start on the block already timed keeps its progress,
/// as vanilla does; a start on another block first clears the old cracks.
fn start(
    scheduler: &mut Scheduler,
    store: &Arc<ChunkStore>,
    rules: DigRules<'_>,
    id: PlayerId,
    position: Position,
    sequence: i32,
) {
    let Some(step) = step_per_tick(store, rules, position) else {
        stop_dig(scheduler, id, None);
        ack(scheduler, id, sequence);
        return;
    };
    if dig::current(scheduler, id).is_some_and(|dig| dig.position == position) {
        ack(scheduler, id, sequence);
        return;
    }
    dig::start(
        scheduler,
        id,
        DigState {
            position,
            progress: 0.0,
            step,
            stage: None,
            ticks: 0,
        },
    );
    ack(scheduler, id, sequence);
}

/// Progress one tick of a bare-handed dig adds, or `None` when the block
/// cannot be timed (air, unloaded, missing from the report, instant or
/// unbreakable).
fn step_per_tick(store: &Arc<ChunkStore>, rules: DigRules<'_>, position: Position) -> Option<f32> {
    let state = store.block(position.x, position.y, position.z)?;
    if store.is_air(state) {
        return None;
    }
    let report = rules.hardness?;
    let hardness = report.hardness(state)?;
    if hardness <= 0.0 {
        // `hardness = 0` is instant (trust the finish); a negative value is
        // unbreakable and never gets a timer. A NaN hardness falls through and
        // is rejected by the finiteness check below.
        return None;
    }
    let speed = report.hand_speed(state).unwrap_or(1.0);
    // A bare hand is never the right tool: vanilla divides the progress by
    // 100 for a state that requires one instead of by 30 (Шаг 17a).
    let divisor = if report.requires_tool(state) {
        TOOL_REQUIRED_DIVISOR
    } else {
        PROGRESS_DIVISOR
    };
    let step = speed as f32 / hardness as f32 / divisor;
    (step.is_finite() && step > 0.0).then_some(step)
}

/// A finish: removes the block if the timer allows it, then acknowledges.
///
/// The success path follows the trace: the `0xFF` crack reset to the
/// observers, `level_event` (the block-break effect, `data` = the state id the
/// block had before removal) to the observers — the digger excluded — then
/// `block_update` to every player (the digger included), then
/// `block_changed_ack` to the digger.
///
/// Two refusals are **hypotheses** (the traces never show one), both answered
/// with the real state to the digger so its prediction is corrected and with
/// the crack reset to the observers:
///
/// - a block the report marks unbreakable (`hardness < 0`) is never removed;
/// - a finish that arrives while the timer is more than
///   [`DigRules::tolerance_ticks`] before `1.0` (the soft tolerance of the
///   plan §6 п.8: the traces show progress 0.94…1.36 at the reset).
///
/// The time check is skipped while the digger is in a state the bare-hand
/// ground model does not describe — airborne or standing in water
/// ([`steady_state`]). The check is a *lower* bound on the elapsed time, so a
/// client that is slower there (vanilla divides its mining speed by five) would
/// pass it anyway; the skip is a safety net against the opposite error, a
/// client whose tick catch-up compresses the finish below the bound (observed
/// live in the 2026-11-17_0131 run). Trusting the client in those states is the
/// plan's option (b). The real divergence is the **stage cadence**: we keep
/// broadcasting at the ground `speed/hardness/30` while vanilla's crack is up
/// to five times slower there, and this is recorded as debt.
///
/// A finish for a block with no timed dig (no report, unknown state, instant
/// block, or a different position) falls back to the pre-16a.3 trust path.
fn finish(
    scheduler: &mut Scheduler,
    store: &Arc<ChunkStore>,
    rules: DigRules<'_>,
    id: PlayerId,
    position: Position,
    sequence: i32,
) {
    let Some(state) = store.block(position.x, position.y, position.z) else {
        // The chunk is not loaded (or y is outside the slots): no state
        // exists to send back, so only the ack goes out.
        stop_dig(scheduler, id, None);
        ack(scheduler, id, sequence);
        return;
    };
    if rules
        .hardness
        .is_some_and(|report| report.is_unbreakable(state))
    {
        refuse(scheduler, id, position, state, sequence);
        return;
    }
    if store.is_air(state) {
        // Already air: nothing changes; the digger is told the real state.
        refuse(scheduler, id, position, state, sequence);
        return;
    }
    let dig = dig::take(scheduler, id);
    if let Some(dig) = dig {
        dig::reset(scheduler, id, dig.position);
        // A dig made airborne or in water is not judged by the ground step:
        // the client is slower there, so its answer would pass anyway, but a
        // tick catch-up can also compress it below the bound (see above).
        let steady = steady_state(scheduler, id, store);
        let too_early = rules.tolerance_ticks.is_some_and(|tolerance| {
            steady && dig.position == position && dig.progress + tolerance as f32 * dig.step < 1.0
        });
        if too_early {
            debug!(
                player = id,
                x = position.x,
                y = position.y,
                z = position.z,
                progress = dig.progress,
                ticks = dig.ticks,
                "finish refused: the timer is not at the expected time"
            );
            deliver(
                scheduler,
                id,
                ServerMessage::BlockUpdate { position, state },
            );
            ack(scheduler, id, sequence);
            return;
        }
    }
    match store.set_block(position.x, position.y, position.z, AIR_STATE) {
        Ok(_) => {
            broadcast_break(scheduler, id, position, state);
            ack(scheduler, id, sequence);
        }
        Err(err) => {
            // The read above already proved the cell is writable; a failure
            // here is a race or malformed data, never a guess.
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

/// Whether the digger is in the state the bare-hand ground model describes:
/// on the ground and not standing in water.
///
/// Vanilla's `Player.getDestroySpeed` divides the mining speed by five while
/// the player is airborne and again while it is in water, so a dig there takes
/// up to `5 × 30·hardness` ticks. The five vanilla digs of 2026-11-14 are all
/// on dry ground (`docs/BLOCK-BREAKING.md` §1.7), so no trace measures that
/// penalty yet. A slower dig passes the lower-bound check by itself; skipping
/// it in those states (plan §6 п.8, option b) protects against a client tick
/// catch-up that compresses the finish. The stages still follow the ground
/// cadence, which is the actual **debt**, not parity.
///
/// The state comes from the last movement packet the client sent
/// ([`PlayerPos::on_ground`](super::region::PlayerPos::on_ground)) and the
/// store's fluid surface at the feet ([`ChunkStore::feet_in_water`]) — the same
/// pair the swimming gate uses. A player missing from the regions (already
/// disconnected) counts as steady, so the check stays on; the dig is dropped
/// with the player anyway.
fn steady_state(scheduler: &Scheduler, id: PlayerId, store: &ChunkStore) -> bool {
    let position = scheduler
        .region_of(id)
        .and_then(|region| scheduler.region(region))
        .and_then(|region| region.get(id))
        .map(|player| player.position());
    let Some(position) = position else {
        return true;
    };
    position.on_ground()
        && !store.feet_in_water(
            position.x.floor() as i32,
            position.y,
            position.z.floor() as i32,
        )
}

/// Aborts the survival dig: clears the cracks and acknowledges.
///
/// The trace shows no abort packet itself (its value `0x02` is inferred), but
/// the reset `0xFF` on an interrupted dig is traced (§1.7).
fn abort(scheduler: &mut Scheduler, id: PlayerId, sequence: i32) {
    stop_dig(scheduler, id, None);
    ack(scheduler, id, sequence);
}

/// Stops any dig `id` was timing and clears its cracks on the observers.
///
/// `keep` names the block a repeated start continues: only the *other* block's
/// cracks are cleared.
fn stop_dig(scheduler: &mut Scheduler, id: PlayerId, keep: Option<Position>) {
    if let Some(dig) = dig::take(scheduler, id) {
        if Some(dig.position) != keep {
            dig::reset(scheduler, id, dig.position);
        }
    }
}

/// Refuses a finish: clears the cracks, sends the real state to the digger and
/// acknowledges. The observers get nothing but the reset — they never saw the
/// block change.
fn refuse(scheduler: &mut Scheduler, id: PlayerId, position: Position, state: i32, sequence: i32) {
    stop_dig(scheduler, id, None);
    deliver(
        scheduler,
        id,
        ServerMessage::BlockUpdate { position, state },
    );
    ack(scheduler, id, sequence);
}

/// Announces one broken block: the break `level_event` to the observers (the
/// digger excluded, per S3/S9), then the `block_update` air state to every
/// player, the digger included. The crack reset goes out before this call
/// (see [`finish`]).
///
/// M4 keeps one region (the whole world is [`SINGLE_REGION`](super::SINGLE_REGION)),
/// so "every player" is the traced audience; the per-chunk view rule is a
/// later refinement (`docs/BLOCK-BREAKING.md` §4.1).
fn broadcast_break(
    scheduler: &mut Scheduler,
    digger: PlayerId,
    position: Position,
    broken_state: i32,
) {
    let ids: Vec<PlayerId> = scheduler
        .regions()
        .iter()
        .flat_map(|region| region.player_ids())
        .collect();
    for id in &ids {
        if *id != digger {
            deliver(
                scheduler,
                *id,
                ServerMessage::LevelEvent {
                    position,
                    state: broken_state,
                },
            );
        }
    }
    for id in ids {
        deliver(
            scheduler,
            id,
            ServerMessage::BlockUpdate {
                position,
                state: AIR_STATE,
            },
        );
    }
}

/// The player's game mode, `None` for an id outside the regions.
fn game_mode(scheduler: &Scheduler, id: PlayerId) -> Option<i32> {
    scheduler
        .regions()
        .iter()
        .find(|region| region.player_ids().any(|player| player == id))
        .and_then(|region| region.get(id))
        .map(|player| player.game_mode())
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
