//! The server-side mining timer (Шаг 16a.3).
//!
//! A survival dig is a per-player state ([`DigState`](super::region::DigState)):
//! its block, the progress added per tick and the stage the observers already
//! saw. The model comes from the vanilla traces
//! (`docs/BLOCK-BREAKING.md` §1.7) and the exported hardness scale:
//!
//! - progress grows by `speed/hardness/30` per tick, so a bare-handed dig
//!   lasts `30·hardness` ticks (`speed = 1.0` in `reports/hardness.json`);
//! - after the increment the stage is `floor(progress·10)`, **not clamped**
//!   (the traces carry 10, 11 and 13), and a packet goes out only when the
//!   byte changes — leaves (6 ticks) therefore start at stage 1, while dirt,
//!   grass and the log start at stage 0;
//! - `0xFF` is a reset: it stops the cracks on completion and on an abort;
//! - the stages reach the **observers only**. The digger draws its own
//!   prediction and the traces show no destruction packet coming back to it;
//! - the server never removes the block by progress: the client's
//!   `player_action` finish does that (16a.2). The timer only decides whether
//!   that finish is plausibly on time.
//!
//! The per-tick advance runs in the game tick after the regions' own tick, so
//! a player who crossed a region boundary this tick is advanced in the region
//! that now owns it. Delivery uses the same `try_send` policy as every other
//! tick → connection message (a full queue kicks the slow player).
//!
//! SOURCE: the five vanilla traces of 2026-11-14 (`docs/BLOCK-BREAKING.md`
//! §1.3/§1.7), `E:\MineRust-data\reports\hardness.json`. The finish-time
//! tolerance is a **hypothesis** (see [`DigRules`]).

use data::BlockHardness;
use protocol::Position;
use tracing::warn;

use super::messages::{PlayerId, ServerMessage};
use super::region::{DigState, Player};
use super::scheduler::Scheduler;

/// Stage byte that removes the cracks: the reset the traces carry on an abort
/// and on completion.
pub const RESET: u8 = 0xFF;

/// Tick divisor of the dig formula: progress grows by `speed/hardness/30` per
/// tick (confirmed for leaves, dirt and the dark oak log in 16a.2). Kept in
/// `f32`: the game accumulates the progress as a float, and the rounding is
/// what makes six `1/6` ticks reach the traced stage `10`.
pub const PROGRESS_DIVISOR: f32 = 30.0;

/// Highest stage byte a progress may produce; `0xFF` is reserved for
/// [`RESET`]. The traces stop at 13, far below it.
const MAX_STAGE: f32 = 254.0;

/// How strictly a survival finish is timed (Шаг 16a.3).
///
/// With `hardness = None` the whole timer is off and the client is trusted
/// about when a block is broken (the pre-16a.3 behaviour). The tolerance is a
/// **hypothesis**: the traces show a finish arriving with progress 0.94…1.36
/// of the expected time, so the default accepts a finish that is up to 3
/// ticks early and never rejects a late one. `tolerance_ticks = None` turns
/// the time check off while keeping the stage broadcasts.
#[derive(Debug, Clone, Copy)]
pub struct DigRules<'a> {
    /// Hardness and bare-hand speed report; `None` keeps the timer off.
    pub hardness: Option<&'a BlockHardness>,
    /// Ticks of slack before the expected finish; `None` trusts every finish.
    pub tolerance_ticks: Option<u32>,
}

impl DigRules<'_> {
    /// Rules that trust the client completely (the creative instant break and
    /// the no-report path). No timer starts and no finish is checked.
    pub fn trusting() -> Self {
        Self {
            hardness: None,
            tolerance_ticks: None,
        }
    }
}

/// The stage byte of a progress: `floor(progress·10)`, unclamped except for
/// the reset sentinel (which a real dig never reaches: the traces stop at 13).
pub fn stage(progress: f32) -> u8 {
    (progress * 10.0).floor().clamp(0.0, MAX_STAGE) as u8
}

/// Advances every timed dig by one tick and announces the stages that changed
/// to the observers of each dig.
pub(super) fn tick(scheduler: &mut Scheduler) {
    let mut changes = Vec::new();
    for region in scheduler.regions_mut() {
        let ids: Vec<PlayerId> = region.player_ids().collect();
        for id in ids {
            let Some(player) = region.get_mut(id) else {
                continue;
            };
            let Some(dig) = player.dig_mut() else {
                continue;
            };
            dig.progress += dig.step;
            dig.ticks = dig.ticks.saturating_add(1);
            let next = stage(dig.progress);
            if dig.stage != Some(next) {
                dig.stage = Some(next);
                changes.push((id, dig.position, next));
            }
        }
    }
    for (digger, position, stage) in changes {
        broadcast(scheduler, digger, position, stage);
    }
}

/// Starts timing a dig for `id`.
///
/// A start on another block while a dig is still timed (the client skipped the
/// abort) first clears the old cracks; the traces always carry an abort before
/// the next start, so this is a **hypothesis** that only edges the malformed
/// case.
pub(super) fn start(scheduler: &mut Scheduler, id: PlayerId, dig: DigState) {
    if let Some(old) = take(scheduler, id) {
        if old.position != dig.position {
            broadcast(scheduler, id, old.position, RESET);
        }
    }
    if let Some(player) = player_mut(scheduler, id) {
        player.set_dig(Some(dig));
    }
}

/// Stops timing the dig of `id`, returning what was tracked.
pub(super) fn take(scheduler: &mut Scheduler, id: PlayerId) -> Option<DigState> {
    player_mut(scheduler, id).and_then(Player::take_dig)
}

/// The dig `id` is timing, if any (a repeated start continues it).
pub(super) fn current(scheduler: &Scheduler, id: PlayerId) -> Option<DigState> {
    let region = scheduler.region_of(id)?;
    scheduler.region(region)?.get(id)?.dig()
}

/// Tells the observers of `digger` that the cracks on `position` are gone.
pub(super) fn reset(scheduler: &mut Scheduler, digger: PlayerId, position: Position) {
    broadcast(scheduler, digger, position, RESET);
}

/// Sends one stage change to every player but the digger.
fn broadcast(scheduler: &mut Scheduler, digger: PlayerId, position: Position, stage: u8) {
    let ids: Vec<PlayerId> = scheduler
        .regions()
        .iter()
        .flat_map(|region| region.player_ids())
        .collect();
    for id in ids {
        if id == digger {
            continue;
        }
        let message = ServerMessage::BlockDestruction {
            digger,
            position,
            stage,
        };
        if let Some(kick) = scheduler.deliver(id, message) {
            warn!(
                player = kick.id,
                reason = kick.reason.message(),
                "kicking a slow player"
            );
        }
    }
}

/// The player's entry in its region, if it is still connected.
fn player_mut(scheduler: &mut Scheduler, id: PlayerId) -> Option<&mut Player> {
    let region = scheduler.region_of(id)?;
    scheduler.region_mut(region)?.get_mut(id)
}

#[cfg(test)]
mod tests {
    use super::stage;

    /// The traced stage bytes: leaves go 1, 3, 5, 6, 8, 10, 11, 13 (progress
    /// may pass 1.0), dirt starts at 0, and the reset byte is never produced
    /// by a real progress.
    #[test]
    fn stages_follow_the_floor_of_ten_times_progress() {
        assert_eq!(stage(0.0), 0, "a fresh dig is stage 0");
        assert_eq!(stage(0.05), 0, "dirt's first tick rounds down");
        assert_eq!(stage(0.17), 1, "leaves' first tick");
        assert_eq!(stage(0.35), 3);
        assert_eq!(stage(0.55), 5);
        assert_eq!(stage(0.99), 9);
        assert_eq!(stage(1.0), 10, "the stage is not clamped to 9");
        assert_eq!(stage(1.36), 13, "the traced maximum");
        assert_eq!(stage(100.0), 254, "the reset byte stays reserved");
    }
}
