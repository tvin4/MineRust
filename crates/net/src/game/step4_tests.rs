//! Шаг 4 (а) and (б): socket-free, in-process determinism and visibility
//! invariants of the M4.5 server work.
//!
//! A single 4-player scenario (entry, walking toward each other, leaving the
//! view, returning, quitting) is driven straight through [`Scheduler`] — no
//! sockets — and replayed at several pool sizes. The tests assert:
//!
//! * (а) the outgoing transcript is identical at every `K` (`0`, `1`, `4`,
//!   `8`) and on a repeat, both with `region_side = 0` and with a small region
//!   tile (so visibility crosses region boundaries); the visibility events do
//!   not depend on the region split at all;
//! * (б) on **every** tick visibility is symmetric and equals its ground truth
//!   (`mutually_visible` over the final positions), no player is lost or
//!   duplicated across regions, and the split scenario really crosses region
//!   boundaries.
//!
//! See `docs/ARCH-M4.md` §6 «M4.5 (сервер) — Шаг 4».

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use super::entities::mutually_visible;
use super::test_support::{join_view, pos_move, pos_rot_move};
use super::*;

/// View distance of every actor: radius `2 × 16 = 32` blocks.
const VIEW: i32 = 2;

/// Region tile side (2 chunks = 32 blocks) that splits the world right at the
/// view boundary, so a player leaving the view also crosses a region border.
const SMALL_SIDE: i32 = 2;

/// A movement a player sends in a tick.
#[derive(Debug, Clone, Copy)]
enum Action {
    /// Absolute `move_player_pos` (x, y, z).
    Pos(f64, f64, f64),
    /// Absolute `move_player_pos_rot` (x, y, z, yaw, pitch).
    PosRot(f64, f64, f64, f32, f32),
}

impl Action {
    /// The movement payload the connection would hand the game thread.
    fn state(self) -> MoveState {
        match self {
            Action::Pos(x, y, z) => pos_move(x, y, z),
            Action::PosRot(x, y, z, yaw, pitch) => pos_rot_move(x, y, z, yaw, pitch, 0x01),
        }
    }
}

/// One tick of the scenario: the moves to queue, then the ids that quit before
/// the tick (a quit is applied in the region that owns the player then).
struct Frame {
    moves: Vec<(PlayerId, Action)>,
    quits: Vec<PlayerId>,
}

/// Builds a [`Frame`].
fn frame(moves: Vec<(PlayerId, Action)>, quits: Vec<PlayerId>) -> Frame {
    Frame { moves, quits }
}

/// The scenario every test drives: entry is the four joins, then these ticks.
fn scenario_frames() -> Vec<Frame> {
    use Action::{Pos, PosRot};
    vec![
        // t0: everyone walks forward, still inside the 32-block view; player 4
        // crosses into the next region tile (x = 32) when the world is split.
        frame(
            vec![
                (1, Pos(8.0, 64.0, 4.0)),
                (2, Pos(16.0, 64.0, 4.0)),
                (3, Pos(24.0, 64.0, 4.0)),
                (4, Pos(32.0, 64.0, 4.0)),
            ],
            vec![],
        ),
        // t1: the pairs walk toward each other.
        frame(
            vec![
                (1, Pos(13.0, 64.0, 4.0)),
                (2, Pos(15.0, 64.0, 4.0)),
                (3, Pos(25.0, 64.0, 4.0)),
                (4, Pos(27.0, 64.0, 4.0)),
            ],
            vec![],
        ),
        // t2: rotation in place — the С4 shape (no position change).
        frame(
            vec![
                (1, PosRot(13.0, 64.0, 4.0, 90.0, 10.0)),
                (2, PosRot(15.0, 64.0, 4.0, -90.0, -10.0)),
                (3, PosRot(25.0, 64.0, 4.0, 45.0, 5.0)),
                (4, PosRot(27.0, 64.0, 4.0, 180.0, -5.0)),
            ],
            vec![],
        ),
        // t3: leave the view in two groups; with a small tile each group lands
        // in a different region.
        frame(
            vec![
                (1, Pos(-20.0, 64.0, 4.0)),
                (2, Pos(-18.0, 64.0, 4.0)),
                (3, Pos(52.0, 64.0, 4.0)),
                (4, Pos(54.0, 64.0, 4.0)),
            ],
            vec![],
        ),
        // t4: keep drifting apart.
        frame(
            vec![
                (1, Pos(-21.0, 64.0, 4.0)),
                (2, Pos(-19.0, 64.0, 4.0)),
                (3, Pos(53.0, 64.0, 4.0)),
                (4, Pos(55.0, 64.0, 4.0)),
            ],
            vec![],
        ),
        // t5: return into the view.
        frame(
            vec![
                (1, Pos(14.0, 64.0, 4.0)),
                (2, Pos(16.0, 64.0, 4.0)),
                (3, Pos(24.0, 64.0, 4.0)),
                (4, Pos(26.0, 64.0, 4.0)),
            ],
            vec![],
        ),
        // t6: one more move.
        frame(
            vec![
                (1, Pos(15.0, 64.0, 4.0)),
                (2, Pos(17.0, 64.0, 4.0)),
                (3, Pos(25.0, 64.0, 4.0)),
                (4, Pos(27.0, 64.0, 4.0)),
            ],
            vec![],
        ),
        // t7..t9: everyone quits, one group at a time.
        frame(vec![], vec![4]),
        frame(vec![], vec![3]),
        frame(vec![], vec![1, 2]),
    ]
}

/// The ids that start the scenario, in join order.
const ACTORS: [PlayerId; 4] = [1, 2, 3, 4];

/// One player as the invariants read it back.
struct View {
    id: PlayerId,
    position: PlayerPos,
    radius: f64,
    visible: BTreeSet<PlayerId>,
}

/// A driven scheduler: joins, per-tick actions and the whole run's transcript.
struct Run {
    scheduler: Scheduler,
    slots: BTreeMap<PlayerId, Arc<MoveSlot>>,
    transcript: Vec<String>,
    tick: u64,
    transfers: usize,
}

impl Run {
    /// A run over `region_side` with `workers` pool threads; the pool path is
    /// forced even for a single region so every `K > 0` really uses it.
    fn new(region_side: i32, workers: usize) -> Self {
        let mut scheduler = Scheduler::with_grid(
            vec![Region::new(SINGLE_REGION)],
            RegionGrid::new(region_side),
            workers,
        );
        if workers > 0 {
            scheduler.force_dispatch();
        }
        Self {
            scheduler,
            slots: BTreeMap::new(),
            transcript: Vec::new(),
            tick: 0,
            transfers: 0,
        }
    }

    /// Joins a player and confirms its entry teleport, so its moves are trusted.
    fn join(&mut self, id: PlayerId, name: &str, spawn: (i32, i32, i32)) {
        let (join, slot, _receiver) = join_view(id, name, VIEW, spawn);
        self.scheduler.insert_player(join);
        self.scheduler.confirm_teleport(id);
        self.slots.insert(id, slot);
    }

    /// Applies one frame: the quits, then the queued moves, then the tick and
    /// its transcript.
    fn step(&mut self, frame: &Frame) -> TickOutcome {
        let tick = self.tick;
        for id in &frame.quits {
            self.scheduler.remove_player(*id);
        }
        for (id, action) in &frame.moves {
            if let Some(slot) = self.slots.get(id) {
                slot.store(action.state());
            }
        }
        let outcome = self.scheduler.tick(tick, Instant::now());
        self.transfers += outcome.transfers.len();
        self.transcript.extend(outcome.transcript(tick));
        self.tick += 1;
        outcome
    }

    /// Every online player with its position, radius and visible set. Panics
    /// if any id is owned by two regions at once (the duplication invariant).
    fn views(&self) -> Vec<View> {
        let mut seen = BTreeSet::new();
        let mut views = Vec::new();
        for region in self.scheduler.regions() {
            for id in region.player_ids() {
                assert!(seen.insert(id), "player {id} is owned by two regions");
                let player = region.get(id).expect("a listed id exists");
                views.push(View {
                    id,
                    position: player.position(),
                    radius: view_radius(player.view_distance()),
                    visible: player.visible().keys().copied().collect(),
                });
            }
        }
        views
    }
}

/// [а] The transcript does not depend on `K`, on a repeat, or on the region
/// split within a configuration; the visibility events are the same with and
/// without a region split.
#[test]
fn the_scenario_transcript_is_deterministic() {
    for side in [0, SMALL_SIDE] {
        let base = drive(side, 0);
        assert!(
            base.transcript
                .iter()
                .any(|line| line.contains("\"kind\":\"entity\"")),
            "side={side}: the scenario must produce entity events"
        );
        for workers in [1usize, 4, 8] {
            let other = drive(side, workers);
            assert_eq!(
                other.transcript, base.transcript,
                "side={side}: K={workers} must produce the same transcript as K=0"
            );
        }
        let repeat = drive(side, 0);
        assert_eq!(
            repeat.transcript, base.transcript,
            "side={side}: a repeat must produce the same transcript"
        );
    }

    // The visibility events must not depend on the region split: the pass runs
    // on the final, global ownership, so a boundary only reorders region ids.
    let flat = entity_events(&drive(0, 0).transcript);
    let split = entity_events(&drive(SMALL_SIDE, 4).transcript);
    assert_eq!(
        split, flat,
        "visibility across a region boundary must match the unsplit run"
    );
}

/// [б] Every tick of the scenario: the visible set equals its ground truth, the
/// relation is symmetric, no player is lost or duplicated, and the small-tile
/// run really crosses region boundaries.
#[test]
fn visibility_is_symmetric_and_ground_true_every_tick() {
    for side in [0, SMALL_SIDE] {
        let mut run = Run::new(side, 4);
        for (index, id) in ACTORS.iter().enumerate() {
            run.join(*id, &format!("actor-{id}"), (4 + 8 * index as i32, 64, 4));
        }
        let mut alive = ACTORS.len();

        for frame in scenario_frames() {
            let tick = run.tick;
            let outcome = run.step(&frame);
            alive -= frame.quits.len();
            assert_visibility(&run, tick, alive);
            // A quit this tick is removed from the others' view with
            // `left_world = true`; a move back into the view spawns again.
            // Only when a viewer is still online: an empty world has nobody
            // left to be told.
            if !frame.quits.is_empty() && alive > 0 {
                assert!(
                    outcome
                        .entities
                        .iter()
                        .flat_map(|event| &event.remove)
                        .any(|remove| remove.left_world),
                    "side={side} tick {tick}: a quit must drop a left-world entity"
                );
            }
        }

        if side == SMALL_SIDE {
            assert!(
                run.transfers > 0,
                "the small-tile scenario must cross region boundaries"
            );
        } else {
            assert_eq!(run.transfers, 0, "the unsplit grid never transfers");
        }
        assert_eq!(run.tick, scenario_frames().len() as u64);
    }
}

/// Runs the scenario on `region_side` with `workers` pool threads.
fn drive(region_side: i32, workers: usize) -> Run {
    let mut run = Run::new(region_side, workers);
    for (index, id) in ACTORS.iter().enumerate() {
        run.join(*id, &format!("actor-{id}"), (4 + 8 * index as i32, 64, 4));
    }
    for frame in scenario_frames() {
        run.step(&frame);
    }
    run
}

/// Asserts the visibility invariants for one tick against the ground truth.
fn assert_visibility(run: &Run, tick: u64, alive: usize) {
    let views = run.views();
    assert_eq!(views.len(), alive, "tick {tick}: the alive set changed");
    assert_eq!(
        run.scheduler.player_count(),
        alive,
        "tick {tick}: the sum of players per region changed"
    );
    for a in &views {
        let expected: BTreeSet<PlayerId> = views
            .iter()
            .filter(|b| {
                b.id != a.id && mutually_visible(&a.position, a.radius, &b.position, b.radius)
            })
            .map(|b| b.id)
            .collect();
        assert_eq!(a.visible, expected, "tick {tick}: visible set of {}", a.id);
    }
    for a in &views {
        for b in &views {
            if a.id == b.id {
                continue;
            }
            assert_eq!(
                a.visible.contains(&b.id),
                b.visible.contains(&a.id),
                "tick {tick}: visibility between {} and {} is not symmetric",
                a.id,
                b.id
            );
        }
    }
}

/// The entity events of a transcript, per tick, keyed without the region-local
/// fields (`region`, `seq`), so two region splits can be compared.
fn entity_events(transcript: &[String]) -> BTreeMap<u64, Vec<String>> {
    let mut out: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    for line in transcript {
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("kind").and_then(|kind| kind.as_str()) != Some("entity") {
            continue;
        }
        let tick = value
            .get("tick")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if let Some(object) = value.as_object_mut() {
            object.remove("region");
            object.remove("seq");
        }
        out.entry(tick).or_default().push(value.to_string());
    }
    for events in out.values_mut() {
        events.sort();
    }
    out
}
