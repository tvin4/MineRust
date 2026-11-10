//! The scheduler ticks regions (ARCH-M4 §1, M4.3 A2).
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
//! M4.3 ships the mechanism with one region and inline ticking, so the live
//! server behaves as before; M4.4 routes joins to several regions.

use std::time::Instant;

use tracing::warn;

use super::messages::PlayerId;
use super::pool::{Job, Pool};
use super::region::{Region, RegionId, TickOutcome};

/// Ticks every region once per tick, inline or through the region pool.
#[derive(Debug, Default)]
pub struct Scheduler {
    regions: Vec<Region>,
    pool: Option<Pool>,
    /// Tests measure the dispatch on a single region; production always
    /// ticks one region inline (the transfer costs more than it saves).
    #[cfg(test)]
    force_dispatch: bool,
}

impl Scheduler {
    /// A scheduler over the given regions, ticking inline (M4.1 behaviour).
    pub fn new(regions: Vec<Region>) -> Self {
        Self {
            regions,
            pool: None,
            #[cfg(test)]
            force_dispatch: false,
        }
    }

    /// A scheduler with `workers` region threads; `0` means no pool at all.
    ///
    /// A pool that cannot start is logged and skipped: the tick stays
    /// correct, only slower, which is the M4.2 behaviour.
    pub fn with_workers(regions: Vec<Region>, workers: usize) -> Self {
        if workers == 0 {
            return Self::new(regions);
        }
        match Pool::new(workers) {
            Ok(pool) => Self {
                regions,
                pool: Some(pool),
                #[cfg(test)]
                force_dispatch: false,
            },
            Err(err) => {
                warn!(%err, workers, "could not start the region pool; ticking inline");
                Self::new(regions)
            }
        }
    }

    /// The regions, in tick order.
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

    /// The region that owns a player (M4.1: the first that has them).
    pub fn region_of(&self, id: PlayerId) -> Option<RegionId> {
        self.regions
            .iter()
            .find(|region| region.contains(id))
            .map(Region::id)
    }

    /// Total number of players across all regions (log/diagnostics only).
    pub fn player_count(&self) -> usize {
        self.regions.iter().map(Region::player_count).sum()
    }

    /// Number of pool workers, `0` without a pool (diagnostics/tests).
    pub fn worker_count(&self) -> usize {
        self.pool.as_ref().map_or(0, Pool::workers)
    }

    /// Ticks every region once and merges what they produced.
    ///
    /// With a pool and more than one region the results are merged by
    /// `region_id`, so the replay order does not depend on which worker
    /// finished first.
    pub fn tick(&mut self, tick: u64, now: Instant) -> TickOutcome {
        if self.dispatch() {
            self.tick_pooled(tick, now)
        } else {
            self.tick_inline(tick, now)
        }
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
        for region in &mut self.regions {
            let produced = region.tick(tick, now);
            outcome.moves.extend(produced.moves);
            outcome.kicks.extend(produced.kicks);
        }
        outcome
    }

    /// The pooled path: hand every region out, collect all of them back
    /// (the barrier) and merge by `region_id`.
    fn tick_pooled(&mut self, tick: u64, now: Instant) -> TickOutcome {
        let regions = std::mem::take(&mut self.regions);
        let total = regions.len();
        let mut returned: Vec<(Region, TickOutcome)> = Vec::with_capacity(total);
        let mut inline: Vec<Region> = Vec::new();
        if let Some(pool) = self.pool.as_ref() {
            let workers = pool.workers().max(1);
            for (slot, region) in regions.into_iter().enumerate() {
                match pool.send(slot % workers, Job { region, tick, now }) {
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
            let outcome = region.tick(tick, now);
            returned.push((region, outcome));
        }
        // The merge order is the region id, not the completion order.
        returned.sort_by_key(|(region, _)| region.id());
        let mut merged = TickOutcome::default();
        for (region, outcome) in returned {
            merged.moves.extend(outcome.moves);
            merged.kicks.extend(outcome.kicks);
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

    /// Forces the pooled path even for a single region (measurement only).
    #[cfg(test)]
    pub(crate) fn force_dispatch(&mut self) {
        self.force_dispatch = true;
    }
}
