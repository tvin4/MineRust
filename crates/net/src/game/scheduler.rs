//! The scheduler ticks regions and applies the transfers they report
//! (ARCH-M4 §1, M4.3 A2/B).
//!
//! [`Scheduler::tick`] is the one place the tick meets the regions:
//!
//! * **inline** — no pool (`K = 0`) or at most one region: the scheduler
//!   thread ticks the region itself, exactly as M4.1/M4.2 did;
//! * **pooled** — `K >= 1` and more than one region: every region is moved
//!   to a worker ([`super::pool`]), the scheduler waits for all of them (the
//!   barrier is "collect as many results as were dispatched") and merges the
//!   outcomes in `region_id` order, never in completion order.
//!
//! After the barrier the scheduler applies every [`Transfer`] the regions
//! reported: the whole player moves by ownership from its source region to
//! its destination region and the `player_id → region_id` index is updated,
//! so the next tick routes the player to the new region.
//!
//! M4.3 ships the mechanism; `region_side = 0` (default) keeps the whole world
//! in [`SINGLE_REGION`], so the live server behaves as before.

use std::collections::BTreeMap;
use std::time::Instant;

use tracing::warn;

use super::grid::RegionGrid;
use super::messages::{PlayerId, PlayerJoin};
use super::pool::{Job, Pool};
use super::region::{Region, RegionId, TickOutcome};

/// Ticks every region once per tick, inline or through the region pool, then
/// applies the transfers.
#[derive(Debug, Default)]
pub struct Scheduler {
    /// The regions, always sorted by id (binary-searchable, deterministic
    /// merge order).
    regions: Vec<Region>,
    /// `player_id → region_id`; authoritative for routing incoming messages
    /// and updated on every join, quit and transfer.
    owners: BTreeMap<PlayerId, RegionId>,
    pool: Option<Pool>,
    /// The chunk → region grid used to route joins and to detect transfers.
    grid: RegionGrid,
    /// Tests measure the dispatch on a single region; production always
    /// ticks one region inline (the transfer costs more than it saves).
    #[cfg(test)]
    force_dispatch: bool,
}

impl Scheduler {
    /// A scheduler over the given regions, ticking inline (M4.1 behaviour).
    pub fn new(regions: Vec<Region>) -> Self {
        Self::with_grid(regions, RegionGrid::single(), 0)
    }

    /// A scheduler with `workers` region threads; `0` means no pool at all.
    ///
    /// A pool that cannot start is logged and skipped: the tick stays
    /// correct, only slower, which is the M4.2 behaviour.
    pub fn with_workers(regions: Vec<Region>, workers: usize) -> Self {
        Self::with_grid(regions, RegionGrid::single(), workers)
    }

    /// A scheduler with the given chunk → region grid and `workers` threads.
    ///
    /// The regions are sorted by id and the `player_id → region_id` index is
    /// rebuilt from them, so a scheduler built over hand-made regions (tests)
    /// routes like one grown from joins.
    pub fn with_grid(mut regions: Vec<Region>, grid: RegionGrid, workers: usize) -> Self {
        regions.sort_by_key(Region::id);
        let mut owners = BTreeMap::new();
        for region in &regions {
            for id in region.player_ids() {
                owners.insert(id, region.id());
            }
        }
        let pool = if workers == 0 {
            None
        } else {
            match Pool::new(workers) {
                Ok(pool) => Some(pool),
                Err(err) => {
                    warn!(%err, workers, "could not start the region pool; ticking inline");
                    None
                }
            }
        };
        Self {
            regions,
            owners,
            pool,
            grid,
            #[cfg(test)]
            force_dispatch: false,
        }
    }

    /// The regions, in id order.
    pub fn regions(&self) -> &[Region] {
        &self.regions
    }

    /// Looks a region up by id.
    pub fn region(&self, id: RegionId) -> Option<&Region> {
        self.regions.iter().find(|region| region.id() == id)
    }

    /// Looks a region up by id, mutably.
    pub fn region_mut(&mut self, id: RegionId) -> Option<&mut Region> {
        self.regions.iter_mut().find(|region| region.id() == id)
    }

    /// The region that owns a player, from the `player_id → region_id` index.
    pub fn region_of(&self, id: PlayerId) -> Option<RegionId> {
        if let Some(&region) = self.owners.get(&id) {
            return Some(region);
        }
        // Defensive fallback for a scheduler that was grown outside the
        // index (none today): scan the regions rather than lose the player.
        self.regions
            .iter()
            .find(|region| region.contains(id))
            .map(Region::id)
    }

    /// The chunk → region grid of this scheduler.
    pub fn grid(&self) -> RegionGrid {
        self.grid
    }

    /// Registers a joining player in the region of its spawn chunk and returns
    /// that region id.
    pub fn insert_player(&mut self, join: PlayerJoin) -> RegionId {
        let id = join.id;
        let region_id = self.grid.region_of_block(join.spawn.x, join.spawn.z);
        if self.region_or_create(region_id).insert(join).is_some() {
            // The replaced player kept the same id, so the index entry below
            // is still correct for the new one.
            warn!(player = id, "a join reused an id that was still online");
        }
        self.owners.insert(id, region_id);
        region_id
    }

    /// Marks a player's entry teleport as confirmed; returns its region.
    pub fn confirm_teleport(&mut self, id: PlayerId) -> Option<RegionId> {
        let region_id = self.region_of(id)?;
        match self
            .region_mut(region_id)
            .and_then(|region| region.get_mut(id))
        {
            Some(player) => {
                player.confirm_teleport();
                Some(region_id)
            }
            None => None,
        }
    }

    /// Removes a player from its owner region and the index; returns the
    /// region that owned it.
    pub fn remove_player(&mut self, id: PlayerId) -> Option<RegionId> {
        let region_id = self.region_of(id)?;
        let removed = self
            .region_mut(region_id)
            .and_then(|region| region.remove(id));
        if removed.is_some() {
            self.owners.remove(&id);
            Some(region_id)
        } else {
            None
        }
    }

    /// Total number of players across all regions (log/diagnostics only).
    pub fn player_count(&self) -> usize {
        self.regions.iter().map(Region::player_count).sum()
    }

    /// Number of pool workers, `0` without a pool (diagnostics/tests).
    pub fn worker_count(&self) -> usize {
        self.pool.as_ref().map_or(0, Pool::workers)
    }

    /// Ticks every region once, merges what they produced and applies the
    /// boundary crossings the tick reported.
    ///
    /// With a pool and more than one region the results are merged by
    /// `region_id`, so the replay order does not depend on which worker
    /// finished first. Transfers are applied after that barrier, in
    /// `(from, seq)` order — the canonical, deterministic order of a tick.
    pub fn tick(&mut self, tick: u64, now: Instant) -> TickOutcome {
        let mut outcome = if self.dispatch() {
            self.tick_pooled(tick, now)
        } else {
            self.tick_inline(tick, now)
        };
        self.apply_transfers(&mut outcome);
        outcome
    }

    /// Whether this tick goes through the pool; one region stays inline.
    fn dispatch(&self) -> bool {
        if self.pool.is_none() {
            return false;
        }
        #[cfg(test)]
        {
            self.force_dispatch || self.regions.len() > 1
        }
        #[cfg(not(test))]
        {
            self.regions.len() > 1
        }
    }

    /// The M4.1/M4.2 path: tick the regions here, in order.
    fn tick_inline(&mut self, tick: u64, now: Instant) -> TickOutcome {
        let mut outcome = TickOutcome::default();
        let grid = self.grid;
        for region in &mut self.regions {
            let produced = region.tick(tick, now, grid);
            outcome.moves.extend(produced.moves);
            outcome.kicks.extend(produced.kicks);
            outcome.transfers.extend(produced.transfers);
        }
        outcome
    }

    /// The pooled path: hand every region out, collect all of them back
    /// (the barrier) and merge by `region_id`.
    fn tick_pooled(&mut self, tick: u64, now: Instant) -> TickOutcome {
        let regions = std::mem::take(&mut self.regions);
        let total = regions.len();
        let grid = self.grid;
        let mut returned: Vec<(Region, TickOutcome)> = Vec::with_capacity(total);
        let mut inline: Vec<Region> = Vec::new();
        if let Some(pool) = self.pool.as_ref() {
            let workers = pool.workers().max(1);
            for (slot, region) in regions.into_iter().enumerate() {
                match pool.send(
                    slot % workers,
                    Job {
                        region,
                        tick,
                        now,
                        grid,
                    },
                ) {
                    Ok(()) => {}
                    // The worker is gone: tick this region here instead of
                    // losing it.
                    Err(error) => inline.push(error.0.region),
                }
            }
            let dispatched = total.saturating_sub(inline.len());
            for _ in 0..dispatched {
                match pool.collect() {
                    Some(done) => returned.push((done.region, done.outcome)),
                    None => break,
                }
            }
        } else {
            inline = regions;
        }
        for mut region in inline {
            let outcome = region.tick(tick, now, grid);
            returned.push((region, outcome));
        }
        // The merge order is the region id, not the completion order.
        returned.sort_by_key(|(region, _)| region.id());
        let mut merged = TickOutcome::default();
        for (region, outcome) in returned {
            merged.moves.extend(outcome.moves);
            merged.kicks.extend(outcome.kicks);
            merged.transfers.extend(outcome.transfers);
            self.regions.push(region);
        }
        if self.regions.len() != total {
            warn!(
                expected = total,
                returned = self.regions.len(),
                "the region pool lost a region; the tick is incomplete"
            );
        }
        merged
    }

    /// Moves every transferred player, whole, from its source region to its
    /// destination and updates the owner index.
    ///
    /// The order is `(from, seq)` with the player id as the final tie-break,
    /// so applying the same tick twice moves the players in the same order.
    fn apply_transfers(&mut self, outcome: &mut TickOutcome) {
        if outcome.transfers.is_empty() {
            return;
        }
        outcome
            .transfers
            .sort_by_key(|transfer| (transfer.from, transfer.seq, transfer.player));
        for transfer in &outcome.transfers {
            let player = self
                .region_mut(transfer.from)
                .and_then(|region| region.remove(transfer.player));
            let Some(player) = player else {
                warn!(
                    player = transfer.player,
                    from = transfer.from,
                    "a transfer found no player in its source region"
                );
                continue;
            };
            if self.region_or_create(transfer.to).put(player).is_some() {
                warn!(
                    player = transfer.player,
                    to = transfer.to,
                    "a transfer overwrote a player in its destination"
                );
            }
            self.owners.insert(transfer.player, transfer.to);
        }
    }

    /// The region with this id, created (in id order) when it does not exist.
    fn region_or_create(&mut self, id: RegionId) -> &mut Region {
        let index = match self.regions.binary_search_by_key(&id, Region::id) {
            Ok(index) => index,
            Err(index) => {
                self.regions.insert(index, Region::new(id));
                index
            }
        };
        &mut self.regions[index]
    }

    /// Forces the pooled path even for a single region (measurement only).
    #[cfg(test)]
    pub(crate) fn force_dispatch(&mut self) {
        self.force_dispatch = true;
    }
}
