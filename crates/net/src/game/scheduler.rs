//! The scheduler ticks regions (ARCH-M4 §1).
//!
//! M4.1 runs the single region on the game thread itself; the shape is
//! already "tick every region once", so M4.3 only puts a thread pool
//! behind the same call instead of reshaping it.

use std::time::Instant;

use super::messages::PlayerId;
use super::region::{Kick, Region, RegionId};

/// Ticks every region once per tick, in order.
#[derive(Debug, Default)]
pub struct Scheduler {
    regions: Vec<Region>,
}

impl Scheduler {
    /// A scheduler over the given regions (M4.1: one).
    pub fn new(regions: Vec<Region>) -> Self {
        Self { regions }
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

    /// Ticks every region once and collects the kicks they produced.
    pub fn tick(&mut self, tick: u64, now: Instant) -> Vec<Kick> {
        let mut kicks = Vec::new();
        for region in &mut self.regions {
            kicks.extend(region.tick(tick, now));
        }
        kicks
    }
}
