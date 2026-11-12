//! The equivalence test of the M4.5 Шаг 3.5 visibility rewrite: the optimized
//! [`super::entities::visibility_pass`] must produce exactly the same visible
//! sets (and the same events) as the naive reference kept in
//! [`super::entities_reference`].

use std::collections::{BTreeMap, BTreeSet};

use super::entities::{view_radius, visibility_pass};
use super::entities_reference::naive_visibility_pass;
use super::test_support::join_view;
use super::{MoveUpdate, PlayerId, Region, SINGLE_REGION};

/// A tiny deterministic xorshift, so the test needs no dependency and a
/// failure is reproducible.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }

    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }
}

/// One player of a random layout.
#[derive(Debug, Clone, Copy)]
struct Spec {
    id: PlayerId,
    view: i32,
    x: f64,
    z: f64,
    yaw: f32,
    pitch: f32,
}

/// Builds one region with every spec at its exact position.
fn build(specs: &[Spec]) -> Vec<Region> {
    let mut region = Region::new(SINGLE_REGION);
    for spec in specs {
        let name = format!("player-{}", spec.id);
        let (join, _slot, _receiver) = join_view(spec.id, &name, spec.view, (0, 64, 0));
        region.insert(join);
        let player = region.get_mut(spec.id).expect("just inserted");
        player.apply_move(&MoveUpdate {
            x: Some(spec.x),
            y: Some(64.0),
            z: Some(spec.z),
            yaw: Some(spec.yaw),
            pitch: Some(spec.pitch),
            flags: 0,
        });
    }
    vec![region]
}

/// The visible pair set of every player, read back from the players.
fn visible_pairs(regions: &[Region]) -> BTreeMap<PlayerId, BTreeSet<PlayerId>> {
    let mut out = BTreeMap::new();
    for region in regions {
        for id in region.player_ids() {
            let player = region.get(id).expect("player");
            out.insert(id, player.visible().keys().copied().collect());
        }
    }
    out
}

/// Applies one move to the same player id in each region set.
fn apply(regions: &mut [&mut Vec<Region>; 2], id: PlayerId, x: f64, z: f64, yaw: f32, pitch: f32) {
    for region_set in regions.iter_mut() {
        let player = region_set[0].get_mut(id).expect("player");
        player.apply_move(&MoveUpdate {
            x: Some(x),
            y: Some(64.0),
            z: Some(z),
            yaw: Some(yaw),
            pitch: Some(pitch),
            flags: 0,
        });
    }
}

#[test]
fn the_optimized_pass_matches_the_naive_reference_on_random_layouts() {
    let mut rng = Rng::new(0x1234_5678_9abc_def0);
    let mut layouts = 0usize;
    for _ in 0..1100 {
        let count = 2 + rng.below(63) as usize; // 2..=64
        let mut specs: Vec<Spec> = Vec::with_capacity(count);
        for k in 0..count {
            let id: PlayerId = k as i32 + 1;
            let view = 1 + rng.below(10) as i32; // 1..=10 chunks, radii 16..=160
            let mut x = rng.range(-500.0, 500.0);
            let mut z = rng.range(-500.0, 500.0);
            // Often place the player exactly on a previous player's radius
            // boundary (and just inside/outside it).
            if k > 0 && rng.below(100) < 40 {
                let previous = specs[rng.below(k as u64) as usize];
                let radius = view_radius(view).min(view_radius(previous.view));
                let angle = rng.range(0.0, std::f64::consts::TAU);
                let offset = [-0.001, 0.0, 0.001, -1.0, 1.0][rng.below(5) as usize];
                let distance = radius + offset;
                x = previous.x + distance * angle.cos();
                z = previous.z + distance * angle.sin();
            }
            specs.push(Spec {
                id,
                view,
                x,
                z,
                yaw: rng.range(-180.0, 180.0) as f32,
                pitch: rng.range(-90.0, 90.0) as f32,
            });
        }
        let mut new_regions = build(&specs);
        let mut naive_regions = build(&specs);
        for round in 0..3 {
            if round > 0 {
                let mut moves: Vec<(PlayerId, f64, f64, f32, f32)> = Vec::new();
                for spec in &specs {
                    if rng.below(2) == 0 {
                        moves.push((
                            spec.id,
                            rng.range(-200.0, 200.0),
                            rng.range(-200.0, 200.0),
                            rng.range(-180.0, 180.0) as f32,
                            rng.range(-90.0, 90.0) as f32,
                        ));
                    }
                }
                let mut both = [&mut new_regions, &mut naive_regions];
                for (id, x, z, yaw, pitch) in moves {
                    apply(&mut both, id, x, z, yaw, pitch);
                }
            }
            let new_events = visibility_pass(&mut new_regions);
            let naive_events = naive_visibility_pass(&mut naive_regions);
            assert_eq!(
                visible_pairs(&new_regions),
                visible_pairs(&naive_regions),
                "layout {layouts} round {round}: the visible pair sets differ"
            );
            assert_eq!(
                new_events, naive_events,
                "layout {layouts} round {round}: the events differ"
            );
        }
        layouts += 1;
    }
    assert!(layouts >= 1000, "at least 1000 layouts: got {layouts}");
}
