//! Scenario tests of the region grid, `Transfer` and the outgoing transcript
//! (M4.3 part B). Socket-free and in-process: the scheduler is driven directly.
//!
//! The tile side is small (2 chunks) so a player crosses a boundary every 32
//! blocks, and every test runs the mechanism at an explicit `K` (`0`, `1`,
//! `4`) instead of relying on the CI core count.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Instant;

use protocol::Position;
use tokio::sync::mpsc;

use super::*;

/// Tile side in chunks of every scenario here.
const SIDE: i32 = 2;

/// A join registration at a block, with the player's movement slot and the
/// connection's side of the outbound queue.
fn join_at(
    id: PlayerId,
    spawn: (i32, i32, i32),
) -> (PlayerJoin, Arc<MoveSlot>, mpsc::Receiver<ServerMessage>) {
    let (outbound, receiver) = mpsc::channel(OUTBOUND_CAPACITY);
    let slot = Arc::new(MoveSlot::default());
    let join = PlayerJoin {
        id,
        name: format!("player-{id}"),
        view_distance: 10,
        spawn: Position {
            x: spawn.0,
            y: spawn.1,
            z: spawn.2,
        },
        connection: PlayerConnection {
            outbound,
            kick: Arc::new(KickFlag::default()),
            last_move: Arc::clone(&slot),
        },
    };
    (join, slot, receiver)
}

/// Payload of `move_player_pos`: three big-endian doubles and the flags byte.
fn pos_move(x: f64, y: f64, z: f64) -> MoveState {
    let mut payload = Vec::with_capacity(25);
    for value in [x, y, z] {
        payload.extend_from_slice(&value.to_be_bytes());
    }
    payload.push(0x01);
    MoveState {
        kind: MoveKind::Pos,
        payload,
    }
}

/// What one player does in one tick of a scenario.
#[derive(Debug, Clone, Copy)]
enum Step {
    /// Send a `move_player_pos` (applied by the tick if the teleport was acked).
    Move((f64, f64, f64)),
    /// Quit before the tick (applied in the owner region at that moment).
    Quit,
}

/// A driven scheduler: joins, per-tick actions, transfers, quits and the
/// outgoing transcript of the whole run.
struct Run {
    scheduler: Scheduler,
    slots: BTreeMap<PlayerId, Arc<MoveSlot>>,
    transfers: Vec<(u64, Vec<Transfer>)>,
    quits: Vec<(u64, PlayerId, Option<RegionId>)>,
    transcript: Vec<String>,
    tick: u64,
}

impl Run {
    fn new(workers: usize) -> Self {
        let scheduler = Scheduler::with_grid(
            vec![Region::new(SINGLE_REGION)],
            RegionGrid::new(SIDE),
            workers,
        );
        Self {
            scheduler,
            slots: BTreeMap::new(),
            transfers: Vec::new(),
            quits: Vec::new(),
            transcript: Vec::new(),
            tick: 0,
        }
    }

    /// Joins a player at `spawn`; `ack` confirms the entry teleport so the
    /// next move is trusted. Returns the region that owns the spawn chunk.
    fn join(&mut self, id: PlayerId, spawn: (i32, i32, i32), ack: bool) -> RegionId {
        let (join, slot, _receiver) = join_at(id, spawn);
        let region = self.scheduler.insert_player(join);
        if ack {
            self.scheduler.confirm_teleport(id);
        }
        self.slots.insert(id, slot);
        region
    }

    /// Applies one tick: the queued actions first (as `apply_queued` does),
    /// then the tick, then the transcript of its results.
    fn step(&mut self, actions: &[(PlayerId, Step)]) -> TickOutcome {
        let tick = self.tick;
        for (id, action) in actions {
            match action {
                Step::Move((x, y, z)) => {
                    if let Some(slot) = self.slots.get(id) {
                        slot.store(pos_move(*x, *y, *z));
                    }
                }
                Step::Quit => {
                    let region = self.scheduler.remove_player(*id);
                    self.quits.push((tick, *id, region));
                }
            }
        }
        let outcome = self.scheduler.tick(tick, Instant::now());
        self.transcript.extend(outcome.transcript(tick));
        self.transfers.push((tick, outcome.transfers.clone()));
        self.tick += 1;
        outcome
    }

    fn all_transfers(&self) -> Vec<Transfer> {
        self.transfers
            .iter()
            .flat_map(|(_, transfers)| transfers.iter().copied())
            .collect()
    }

    fn transferred_in_tick(&self, tick: u64) -> bool {
        self.transfers
            .iter()
            .any(|(at, transfers)| *at == tick && !transfers.is_empty())
    }

    fn contains(&self, id: PlayerId, region: RegionId) -> bool {
        self.scheduler
            .region(region)
            .is_some_and(|region| region.contains(id))
    }

    /// Every id the regions hold, each exactly once; the size is the player
    /// count, which the caller compares against the expected alive set.
    fn owned_ids(&self) -> BTreeSet<PlayerId> {
        let mut seen = BTreeSet::new();
        for region in self.scheduler.regions() {
            for id in region.player_ids() {
                assert!(seen.insert(id), "player {id} is in two regions");
            }
        }
        seen
    }
}

#[test]
fn a_region_visits_players_in_id_order_regardless_of_insertion_order() {
    // A `HashMap` would iterate in a per-process random order; the `BTreeMap`
    // of `Region` must make the moved ids strictly ascending whatever order
    // the players were inserted in (M4.3 B1.4).
    let grid = RegionGrid::new(SIDE);
    let moved_ids = |order: [PlayerId; 3]| {
        let mut region = Region::new(SINGLE_REGION);
        let mut slots = Vec::new();
        for id in order {
            let (join, slot, _receiver) = join_at(id, (1, 64, 1));
            region.insert(join);
            region.get_mut(id).expect("inserted").confirm_teleport();
            slots.push((id, slot));
        }
        for (id, slot) in &slots {
            slot.store(pos_move(f64::from(*id), 64.0, 1.0));
        }
        let outcome = region.tick(0, Instant::now(), grid);
        outcome
            .moves
            .iter()
            .map(|moved| moved.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(moved_ids([3, 1, 2]), vec![1, 2, 3]);
    assert_eq!(moved_ids([2, 3, 1]), vec![1, 2, 3]);
}

/// B2 (а): a player walking straight across a boundary.
#[test]
fn a_player_crossing_a_boundary_transfers_and_moves_on_in_the_new_region() {
    for workers in [0usize, 1, 4] {
        let mut run = Run::new(workers);
        let region_a = run.join(1, (30, 64, 5), true);
        assert_eq!(region_a, SINGLE_REGION, "the spawn chunk is region 0");

        // Tick 0: a move that stays inside region A, no crossing.
        let inside = run.step(&[(1, Step::Move((31.0, 64.0, 5.0)))]);
        assert_eq!(inside.moves.len(), 1);
        assert!(inside.transfers.is_empty(), "still inside region A");

        // Tick 1: the move lands in the next tile (x >= 32).
        let crossing = run.step(&[(1, Step::Move((40.0, 64.0, 5.0)))]);
        assert_eq!(
            crossing.moves.len(),
            1,
            "the move is applied in the source region"
        );
        assert_eq!(
            crossing.moves[0].region, region_a,
            "the source region applies it"
        );
        assert_eq!(
            crossing.moves[0].position.x, 40.0,
            "the position is continuous"
        );
        assert_eq!(crossing.transfers.len(), 1, "the crossing is reported");
        let transfer = crossing.transfers[0];
        assert_eq!(transfer.player, 1);
        assert_eq!(transfer.from, region_a);
        assert_ne!(transfer.to, region_a);
        let region_b = transfer.to;
        assert!(run.contains(1, region_b), "the player now belongs to B");
        assert!(!run.contains(1, region_a));

        // Tick 2: the next move is applied in region B.
        let following = run.step(&[(1, Step::Move((48.0, 64.0, 5.0)))]);
        assert_eq!(following.moves.len(), 1);
        assert_eq!(
            following.moves[0].region, region_b,
            "from the next tick B applies the moves"
        );
        assert_eq!(following.moves[0].position.x, 48.0);
        assert!(!run.transferred_in_tick(2), "already in B, no new crossing");
        let owned = run
            .scheduler
            .region(region_b)
            .and_then(|region| region.get(1))
            .expect("the player is in B");
        assert_eq!(owned.position().x, 48.0, "position stayed continuous");
    }
}

/// Runs B2 (б): four players cross boundaries in one tick, in both
/// directions and into different regions.
fn two_way_scenario(workers: usize) -> Run {
    let mut run = Run::new(workers);
    run.join(1, (30, 64, 5), true); // region A (0,0)
    run.join(2, (30, 64, 7), true); // region A
    run.join(3, (34, 64, 5), true); // region B (1,0)
    run.join(4, (30, 64, 40), true); // region C (0,1)
    run.step(&[
        (1, Step::Move((40.0, 64.0, 5.0))),  // A -> B
        (2, Step::Move((40.0, 64.0, 7.0))),  // A -> B
        (3, Step::Move((20.0, 64.0, 5.0))),  // B -> A
        (4, Step::Move((40.0, 64.0, 40.0))), // C -> D
    ]);
    run
}

#[test]
fn several_players_cross_in_one_tick_in_from_seq_order() {
    let run = two_way_scenario(0);
    let transfers = run.all_transfers();
    assert_eq!(transfers.len(), 4, "one crossing per player");
    // Source regions ascending (A=0, C=1, B=65536) with the player id as the
    // within-region order, so A's two crossings come first, then C, then B.
    let order: Vec<PlayerId> = transfers.iter().map(|transfer| transfer.player).collect();
    assert_eq!(order, vec![1, 2, 4, 3]);
    let mut sorted = transfers.clone();
    sorted.sort_by_key(|transfer| (transfer.from, transfer.seq));
    assert_eq!(transfers, sorted, "already canonical");
}

#[test]
fn the_two_way_transcript_is_the_same_at_every_k() {
    let base = two_way_scenario(0);
    assert!(!base.transcript.is_empty(), "the scenario produced results");
    for workers in [1usize, 4] {
        let other = two_way_scenario(workers);
        assert_eq!(
            other.transcript, base.transcript,
            "K={workers} must produce the same transcript"
        );
    }
}

/// B2 (в): where a Quit is routed depends on when it arrives relative to the
/// transfer.
#[test]
fn a_quit_is_routed_to_the_region_that_owns_the_player_at_that_moment() {
    let mut run = Run::new(0);
    let region_a = run.join(1, (30, 64, 5), true);
    run.join(2, (30, 64, 9), true);

    // Tick 0: player 1 crosses into B; player 2 quits while still in A.
    let crossing = run.step(&[(1, Step::Move((40.0, 64.0, 5.0))), (2, Step::Quit)]);
    assert_eq!(crossing.transfers.len(), 1);
    let region_b = crossing.transfers[0].to;

    // The quit of player 2 in that same tick went to the old region A.
    let quit_2 = run
        .quits
        .iter()
        .find(|(_, id, _)| *id == 2)
        .expect("player 2 quit");
    assert_eq!(quit_2.2, Some(region_a), "quit before the transfer -> A");

    // Tick 1: player 1 quits from its new region B.
    run.step(&[(1, Step::Quit)]);
    let quit_1 = run
        .quits
        .iter()
        .find(|(_, id, _)| *id == 1)
        .expect("player 1 quit");
    assert_eq!(quit_1.0, 1, "quit happened in tick 1");
    assert_eq!(quit_1.2, Some(region_b), "quit after the transfer -> B");
    assert!(run.owned_ids().is_empty(), "both players are gone");
}

/// B2 (г): an unconfirmed player is never transferred (its moves are
/// ignored), and a transferred player keeps its confirmation.
#[test]
fn a_transfer_keeps_the_confirmation_and_an_unconfirmed_player_is_ignored() {
    let mut run = Run::new(0);
    let region_a = run.join(1, (30, 64, 5), true); // confirmed
    run.join(2, (30, 64, 9), false); // never confirmed

    let outcome = run.step(&[
        (1, Step::Move((40.0, 64.0, 5.0))),
        (2, Step::Move((40.0, 64.0, 9.0))),
    ]);

    assert_eq!(outcome.moves.len(), 1, "only the confirmed player moved");
    assert_eq!(outcome.moves[0].id, 1);
    assert_eq!(
        outcome.transfers.len(),
        1,
        "only the confirmed player crossed"
    );
    let region_b = outcome.transfers[0].to;

    let moved = run
        .scheduler
        .region(region_b)
        .and_then(|region| region.get(1))
        .expect("the confirmed player is in B");
    assert!(
        moved.teleport_confirmed(),
        "the confirmation travels with the player"
    );

    assert!(
        run.contains(2, region_a),
        "the unconfirmed player stayed in A"
    );
    let unmoved = run
        .scheduler
        .region(region_a)
        .and_then(|region| region.get(2))
        .expect("the unconfirmed player is in A");
    assert!(
        !unmoved.teleport_confirmed(),
        "it is still unconfirmed after the transfer tick"
    );
    assert_eq!(
        (unmoved.position().x, unmoved.position().z),
        (30.5, 9.5),
        "its ignored move did not change the position"
    );
}

/// B2 (д): five players cross several boundaries in a row; the sum of players
/// per region always equals the alive set, with no loss or duplication.
#[test]
fn crossing_several_boundaries_loses_no_player() {
    let mut run = Run::new(0);
    // id, x, z, dx, dz: walking in every direction across the 32-block tiles.
    let mut paths: Vec<(PlayerId, f64, f64, f64, f64)> = vec![
        (1, 4.0, 4.0, 12.0, 0.0),
        (2, 4.0, 36.0, 12.0, 0.0),
        (3, 60.0, 4.0, -12.0, 0.0),
        (4, 20.0, 60.0, 4.0, 12.0),
        (5, 50.0, 50.0, -8.0, 8.0),
    ];
    for (id, x, z, _, _) in &paths {
        run.join(*id, (*x as i32, 64, *z as i32), true);
    }

    let mut total_transfers = 0usize;
    for _ in 0..10 {
        for (_, x, z, dx, dz) in paths.iter_mut() {
            *x += *dx;
            *z += *dz;
        }
        let actions: Vec<(PlayerId, Step)> = paths
            .iter()
            .map(|(id, x, z, _, _)| (*id, Step::Move((*x, 64.0, *z))))
            .collect();
        let outcome = run.step(&actions);
        total_transfers += outcome.transfers.len();

        // Conservation: the regions hold each player exactly once, and the
        // sum over the regions is the number of players still online.
        let owned = run.owned_ids();
        assert_eq!(owned.len(), paths.len(), "a player was lost or duplicated");
        assert_eq!(
            run.scheduler.player_count(),
            paths.len(),
            "the sum of players per region changed"
        );
    }

    assert!(
        total_transfers >= 3,
        "several crossings happened, got {total_transfers}"
    );
    assert!(
        run.scheduler.regions().len() >= 4,
        "at least four regions were touched, got {}",
        run.scheduler.regions().len()
    );
}
