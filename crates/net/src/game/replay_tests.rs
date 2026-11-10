//! Canonical outgoing transcript and journal playback (M4.3 part B3).
//!
//! The "outgoing transcript" of a run is the ordered list of everything the
//! ticks produced — the positions they applied, the kicks they raised and the
//! boundary crossings they detected — in `(tick, region, seq)` order. M4.3 has
//! no real outgoing packets yet (`StreamChunks` arrives in M4.4), so this
//! transcript *is* the tick's output; `docs/ARCH-M4.md` §1/§6 says so plainly.
//!
//! These tests drive the real scheduler and the real journal (`super::apply`,
//! `super::record_moves`, `super::record_transfers`), so the buffer the
//! playback reads is exactly the line format the live server writes. The
//! playback applies only input lines (`join`, `move`, `teleport_ack`, `quit`)
//! and must reproduce the live transcript byte for byte.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use super::test_support::{join_at, pos_move, pos_rot_move, SIDE};
use super::*;

/// One player action queued for a tick: applied before the tick (join, ack,
/// quit) or stored in the movement slot the tick reads (move).
#[derive(Debug, Clone, Copy)]
enum Action {
    /// A connection reaches the play state at this block.
    Join(PlayerId, (i32, i32, i32)),
    /// The client confirms the entry teleport.
    Ack(PlayerId),
    /// The connection ended.
    Quit(PlayerId),
    /// The newest `move_player_pos` of this player.
    Move(PlayerId, (f64, f64, f64)),
}

/// The live run: the real scheduler plus the real journal, so the recorded
/// bytes are exactly what `game_loop` writes.
struct LiveRun {
    scheduler: Scheduler,
    replay: Replay<Vec<u8>>,
    slots: BTreeMap<PlayerId, Arc<MoveSlot>>,
    transcript: Vec<String>,
    tick: u64,
}

impl LiveRun {
    fn new(workers: usize) -> Self {
        Self {
            scheduler: Scheduler::with_grid(
                vec![Region::new(SINGLE_REGION)],
                RegionGrid::new(SIDE),
                workers,
            ),
            replay: Replay::new(Some(Vec::new())),
            slots: BTreeMap::new(),
            transcript: Vec::new(),
            tick: 0,
        }
    }

    /// Queues the tick's messages and moves, ticks once, records the applied
    /// events exactly as `game_loop` does and appends the tick's transcript.
    fn step(&mut self, actions: &[Action]) -> TickOutcome {
        let tick = self.tick;
        self.replay.start_tick();
        for action in actions {
            match *action {
                Action::Join(id, spawn) => {
                    let (join, slot, _receiver) = join_at(id, spawn);
                    self.slots.insert(id, slot);
                    let message = ToGame::Join(Box::new(join));
                    super::apply(&mut self.scheduler, &mut self.replay, tick, message);
                }
                Action::Ack(id) => {
                    let message = ToGame::TeleportAck { id, teleport_id: 1 };
                    super::apply(&mut self.scheduler, &mut self.replay, tick, message);
                }
                Action::Quit(id) => {
                    super::apply(
                        &mut self.scheduler,
                        &mut self.replay,
                        tick,
                        ToGame::Quit { id },
                    );
                }
                Action::Move(id, (x, y, z)) => {
                    if let Some(slot) = self.slots.get(&id) {
                        slot.store(pos_move(x, y, z));
                    }
                }
            }
        }
        let outcome = self.scheduler.tick(tick, Instant::now());
        super::record_moves(&mut self.replay, tick, &outcome.moves);
        super::record_transfers(&mut self.replay, tick, &outcome.transfers);
        self.transcript.extend(outcome.transcript(tick));
        self.tick += 1;
        outcome
    }

    /// The run's transcript and the JSONL journal it wrote.
    fn finish(self) -> (Vec<String>, Vec<u8>) {
        let bytes = self
            .replay
            .into_sink()
            .expect("the live journal is enabled");
        (self.transcript, bytes)
    }
}

/// The scenario every determinism test replays: joins in two distant tiles,
/// four crossings in one tick (both directions), a crossing back, a quit in
/// the same tick as moves, and a walk into a third tile column.
fn scenario(run: &mut LiveRun) {
    // Tick 0: joins and teleport confirmations. Two players in tile A (0,0),
    // two in the far tile D (1,1).
    run.step(&[
        Action::Join(1, (30, 64, 5)),
        Action::Join(2, (4, 64, 4)),
        Action::Join(3, (60, 64, 36)),
        Action::Join(4, (34, 64, 60)),
        Action::Ack(1),
        Action::Ack(2),
        Action::Ack(3),
        Action::Ack(4),
    ]);

    // Tick 1: four crossings in one tick, two directions (A -> B, A -> B,
    // D -> C, D -> C). B is tile (1,0), C is tile (0,1).
    run.step(&[
        Action::Move(1, (40.0, 64.0, 5.0)),
        Action::Move(2, (40.0, 64.0, 4.0)),
        Action::Move(3, (30.0, 64.0, 36.0)),
        Action::Move(4, (30.0, 64.0, 60.0)),
    ]);

    // Tick 2: one stays, two cross back.
    run.step(&[
        Action::Move(1, (48.0, 64.0, 5.0)),  // stays in B
        Action::Move(2, (30.0, 64.0, 4.0)),  // B -> A
        Action::Move(3, (30.0, 64.0, 30.0)), // C -> A
    ]);

    // Tick 3: a quit and two moves in the same tick.
    run.step(&[
        Action::Quit(4),
        Action::Move(1, (56.0, 64.0, 5.0)), // stays in B
        Action::Move(2, (40.0, 64.0, 4.0)), // A -> B
    ]);

    // Tick 4: a walk two tiles to the east (B -> E, tile (2,0)).
    run.step(&[Action::Move(1, (72.0, 64.0, 5.0))]);
}

/// Runs the scenario once and returns its transcript and journal.
fn run_scenario(workers: usize) -> (Vec<String>, Vec<u8>) {
    let mut run = LiveRun::new(workers);
    scenario(&mut run);
    run.finish()
}

/// One journal line, as the playback reads it.
#[derive(serde::Deserialize)]
struct Line {
    tick: u64,
    seq: u32,
    /// Region that applied the event; the playback routes by the scheduler
    /// index, so it does not need it (kept for completeness).
    #[allow(dead_code)]
    region: Option<RegionId>,
    src: PlayerId,
    kind: String,
    payload: serde_json::Value,
}

/// Replays a journal into a fresh scheduler and returns the transcript the
/// live run would have produced.
///
/// Only the input kinds are applied (`join`, `move`, `teleport_ack`, `quit`);
/// `transfer` is a tick result and is not fed back. A recorded `move` line
/// carries the absolute applied position, so rebuilding it as a
/// `move_player_pos_rot` is idempotent and reproduces the same position,
/// flags and boundary crossing.
fn playback(journal: &[u8], workers: usize) -> Vec<String> {
    let text = std::str::from_utf8(journal).expect("the journal is UTF-8");
    let mut by_tick: BTreeMap<u64, Vec<Line>> = BTreeMap::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let record: Line = serde_json::from_str(line).expect("a valid journal line");
        by_tick.entry(record.tick).or_default().push(record);
    }

    let mut scheduler = Scheduler::with_grid(
        vec![Region::new(SINGLE_REGION)],
        RegionGrid::new(SIDE),
        workers,
    );
    let mut replay = Replay::new(None::<Vec<u8>>);
    let mut slots: BTreeMap<PlayerId, Arc<MoveSlot>> = BTreeMap::new();
    let mut transcript = Vec::new();

    for (tick, mut lines) in by_tick {
        lines.sort_by_key(|line| line.seq);
        replay.start_tick();
        for line in &lines {
            match line.kind.as_str() {
                "join" => {
                    let spawn = (
                        line.payload["spawn"]["x"].as_i64().unwrap_or(0) as i32,
                        line.payload["spawn"]["y"].as_i64().unwrap_or(0) as i32,
                        line.payload["spawn"]["z"].as_i64().unwrap_or(0) as i32,
                    );
                    let (join, slot, _receiver) = join_at(line.src, spawn);
                    slots.insert(line.src, slot);
                    let message = ToGame::Join(Box::new(join));
                    super::apply(&mut scheduler, &mut replay, tick, message);
                }
                "teleport_ack" => {
                    let teleport_id = line.payload["teleport_id"].as_i64().unwrap_or(0) as i32;
                    let message = ToGame::TeleportAck {
                        id: line.src,
                        teleport_id,
                    };
                    super::apply(&mut scheduler, &mut replay, tick, message);
                }
                "quit" => {
                    super::apply(
                        &mut scheduler,
                        &mut replay,
                        tick,
                        ToGame::Quit { id: line.src },
                    );
                }
                "move" => {
                    let x = line.payload["x"].as_f64().unwrap_or(0.0);
                    let y = line.payload["y"].as_f64().unwrap_or(0.0);
                    let z = line.payload["z"].as_f64().unwrap_or(0.0);
                    let yaw = line.payload["yaw"].as_f64().unwrap_or(0.0) as f32;
                    let pitch = line.payload["pitch"].as_f64().unwrap_or(0.0) as f32;
                    let flags = line.payload["flags"].as_u64().unwrap_or(0) as u8;
                    if let Some(slot) = slots.get(&line.src) {
                        slot.store(pos_rot_move(x, y, z, yaw, pitch, flags));
                    }
                }
                // A tick result, not an input: the crossing is reproduced by
                // replaying the move that caused it.
                "transfer" => {}
                other => panic!("unexpected journal kind {other}"),
            }
        }
        let outcome = scheduler.tick(tick, Instant::now());
        transcript.extend(outcome.transcript(tick));
    }
    transcript
}

/// B3 (а): the same scenario run twice, in two separate schedulers, must
/// produce a byte-identical transcript (and journal).
#[test]
fn the_same_scenario_is_byte_identical_twice() {
    let (first, first_journal) = run_scenario(0);
    let (second, second_journal) = run_scenario(0);

    assert!(!first.is_empty(), "the scenario must produce output");
    assert!(
        first
            .iter()
            .any(|line| line.contains("\"kind\":\"transfer\"")),
        "the transcript must contain a crossing"
    );
    assert!(
        first.iter().any(|line| line.contains("\"kind\":\"move\"")),
        "the transcript must contain an applied move"
    );
    assert_eq!(first, second, "two runs must give the same transcript");
    assert_eq!(
        first_journal, second_journal,
        "two runs must give the same journal"
    );
}

/// B3 (б): the transcript does not depend on the worker count.
#[test]
fn the_transcript_is_independent_of_the_worker_count() {
    let (base, _) = run_scenario(0);
    for workers in [1usize, 4, 8] {
        let (other, _) = run_scenario(workers);
        assert_eq!(
            other, base,
            "K={workers} must produce the same transcript as K=0"
        );
    }
}

/// B3 (в): applying only the recorded journal to a fresh scheduler reproduces
/// the live transcript — no format change needed.
#[test]
fn the_journal_reproduces_the_live_transcript() {
    let (live, journal) = run_scenario(0);
    assert!(
        journal
            .split(|byte| *byte == b'\n')
            .any(|line| !line.is_empty()),
        "the journal must have lines"
    );
    assert_eq!(
        playback(&journal, 0),
        live,
        "the journal playback must match the live transcript"
    );
    for workers in [1usize, 4] {
        assert_eq!(
            playback(&journal, workers),
            live,
            "the journal playback must match at K={workers} too"
        );
    }
}
