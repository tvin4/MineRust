//! Tests of the game thread's timing and bookkeeping.

use std::sync::Arc;
use std::time::{Duration, Instant};

use protocol::Position;
use tokio::sync::mpsc;

use super::*;

/// A join registration with a queue of the given capacity, the connection's
/// side of that queue, the player's kick flag and movement slot.
fn join_fixture_with(
    id: PlayerId,
    capacity: usize,
) -> (
    PlayerJoin,
    mpsc::Receiver<ServerMessage>,
    Arc<KickFlag>,
    Arc<MoveSlot>,
) {
    let (outbound, receiver) = mpsc::channel(capacity.max(1));
    let kick = Arc::new(KickFlag::default());
    let last_move = Arc::new(MoveSlot::default());
    let join = PlayerJoin {
        id,
        name: format!("player-{id}"),
        view_distance: 10,
        spawn: Position::default(),
        game_mode: 0,
        connection: PlayerConnection {
            outbound,
            kick: Arc::clone(&kick),
            last_move: Arc::clone(&last_move),
        },
    };
    (join, receiver, kick, last_move)
}

/// A join registration plus the connection's side of its outbound queue.
fn join_fixture(id: PlayerId) -> (PlayerJoin, mpsc::Receiver<ServerMessage>) {
    let (join, receiver, _kick, _slot) = join_fixture_with(id, OUTBOUND_CAPACITY);
    (join, receiver)
}

/// One small outbound message, so queue slots can be counted.
fn payload(marker: u8) -> ServerMessage {
    ServerMessage::Send {
        id: 0x50,
        payload: vec![marker],
    }
}

#[test]
fn a_region_keeps_confirms_and_removes_a_player() {
    let mut region = Region::new(SINGLE_REGION);
    let (join, _receiver) = join_fixture(1);
    assert!(region.insert(join).is_none(), "a fresh id");
    assert!(region.contains(1));
    assert_eq!(region.player_count(), 1);

    let player = region.get_mut(1).expect("the player is there");
    assert!(!player.teleport_confirmed(), "movement is untrusted first");
    player.confirm_teleport();
    assert!(region.get(1).expect("still there").teleport_confirmed());

    assert!(region.remove(1).is_some());
    assert!(region.is_empty(), "removing the only player empties it");
}

#[test]
fn the_move_slot_keeps_only_the_newest_packet() {
    let slot = MoveSlot::default();
    assert!(slot.take().is_none(), "an untouched slot is empty");
    slot.store(MoveState {
        kind: MoveKind::Pos,
        payload: vec![1, 2, 3],
    });
    slot.store(MoveState {
        kind: MoveKind::Rot,
        payload: vec![4, 5, 6],
    });
    let newest = slot.take().expect("the newest packet");
    assert_eq!(
        newest.kind,
        MoveKind::Rot,
        "the older packet is overwritten"
    );
    assert!(slot.take().is_none(), "taking drains the slot");
}

/// Payload of `move_player_pos`: three big-endian doubles and the flags byte.
fn pos_payload(x: f64, y: f64, z: f64, flags: u8) -> Vec<u8> {
    let mut out = Vec::with_capacity(25);
    for value in [x, y, z] {
        out.extend_from_slice(&value.to_be_bytes());
    }
    out.push(flags);
    out
}

/// A `move_player_pos` slot value standing on the ground (flags 0x01).
fn pos_move(x: f64, y: f64, z: f64) -> MoveState {
    MoveState {
        kind: MoveKind::Pos,
        payload: pos_payload(x, y, z, 0x01),
    }
}

#[test]
fn a_movement_before_the_teleport_ack_is_ignored() {
    let mut region = Region::new(SINGLE_REGION);
    let (join, _receiver, _kick, slot) = join_fixture_with(1, OUTBOUND_CAPACITY);
    let spawn = join.spawn;
    region.insert(join);
    let entry = region.get(1).expect("the player is there").position();

    slot.store(pos_move(100.0, 70.0, -50.0));
    let outcome = region.tick(0, Instant::now(), RegionGrid::single());

    assert!(
        outcome.moves.is_empty(),
        "an unconfirmed teleport is not trusted"
    );
    let position = region.get(1).expect("still there").position();
    assert_eq!(position, entry, "the position did not move");
    assert_eq!(position.x, f64::from(spawn.x) + 0.5, "still at the spawn");
}

#[test]
fn a_movement_after_the_ack_moves_the_player() {
    let mut region = Region::new(SINGLE_REGION);
    let (join, _receiver, _kick, slot) = join_fixture_with(1, OUTBOUND_CAPACITY);
    region.insert(join);
    region.get_mut(1).expect("the player").confirm_teleport();

    slot.store(pos_move(100.5, 70.0, -50.25));
    let outcome = region.tick(7, Instant::now(), RegionGrid::single());

    assert_eq!(outcome.moves.len(), 1, "the move is applied once");
    let applied = outcome.moves[0];
    assert_eq!(applied.region, SINGLE_REGION);
    assert_eq!(applied.id, 1);
    assert_eq!(
        (applied.position.x, applied.position.y, applied.position.z),
        (100.5, 70.0, -50.25)
    );
    assert!(applied.position.on_ground());
    assert_eq!(applied.position.flags, 0x01);

    let position = region.get(1).expect("still there").position();
    assert_eq!(
        position, applied.position,
        "the player keeps the applied position"
    );
}

#[test]
fn a_rotation_packet_keeps_the_position() {
    let mut region = Region::new(SINGLE_REGION);
    let (join, _receiver, _kick, slot) = join_fixture_with(1, OUTBOUND_CAPACITY);
    region.insert(join);
    region.get_mut(1).expect("the player").confirm_teleport();
    let before = region.get(1).expect("the player").position();

    let mut payload = Vec::new();
    payload.extend_from_slice(&90.0f32.to_be_bytes());
    payload.extend_from_slice(&(-45.0f32).to_be_bytes());
    // 0x02 is a real wire value (bit 0 clear): the position must not move and
    // on-ground must stay false even though the byte is non-zero.
    payload.push(0x02);
    slot.store(MoveState {
        kind: MoveKind::Rot,
        payload,
    });

    let outcome = region.tick(0, Instant::now(), RegionGrid::single());
    let after = outcome.moves[0].position;
    assert_eq!(
        (after.x, after.y, after.z),
        (before.x, before.y, before.z),
        "rotation does not move the player"
    );
    assert_eq!(after.yaw, 90.0);
    assert_eq!(after.pitch, -45.0);
    assert_eq!(after.flags, 0x02);
    assert!(!after.on_ground(), "bit 0 is clear, so not on the ground");
}

#[test]
fn a_full_outbound_queue_kicks_the_player_and_frees_the_region() {
    let mut region = Region::new(SINGLE_REGION);
    let (join, _receiver, kick, _slot) = join_fixture_with(7, 1);
    assert!(region.insert(join).is_none(), "a fresh id");

    // Capacity one: the first message fills the queue, the player stays.
    assert!(region.deliver(7, payload(1)).is_none());
    assert!(region.contains(7), "one queued message is not too many");
    assert_eq!(kick.reason(), None, "nothing raised yet");

    // The second message cannot be queued: the player is kicked and gone.
    let kicked = region.deliver(7, payload(2)).expect("a kick");
    assert_eq!(kicked.id, 7);
    assert_eq!(kicked.reason, KickReason::SendQueueOverflow);
    assert!(!region.contains(7), "the kicked player is removed");
    assert_eq!(kick.reason(), Some(KickReason::SendQueueOverflow));
}

#[test]
fn a_closed_queue_is_not_a_kick() {
    let mut region = Region::new(SINGLE_REGION);
    let (join, receiver, kick, _slot) = join_fixture_with(9, 1);
    assert!(region.insert(join).is_none());

    // The connection is gone: its queue is closed, not overflowing.
    drop(receiver);
    assert!(region.deliver(9, payload(1)).is_none());
    assert!(region.contains(9), "a gone connection is not a kick");
    assert_eq!(kick.reason(), None);

    // An unknown player is silently ignored.
    assert!(region.deliver(404, payload(1)).is_none());
}

#[test]
fn a_tick_that_falls_behind_resets_its_deadline() {
    let config = GameConfig {
        period: Duration::from_millis(50),
        reset_after: 5,
        ..GameConfig::default()
    };
    let deadline = Instant::now();

    // Exactly `reset_after` periods of lag is still catch-up, not a reset.
    let at_threshold = deadline + config.period * config.reset_after;
    assert!(overdue_by(at_threshold, deadline, &config).is_none());

    // One millisecond past the threshold the deadline must be reset.
    let over = overdue_by(at_threshold + Duration::from_millis(1), deadline, &config)
        .expect("a reset is due");
    assert_eq!(
        over,
        config.period * config.reset_after + Duration::from_millis(1)
    );

    // A deadline still in the future has no lag at all.
    assert!(overdue_by(deadline, deadline + Duration::from_millis(10), &config).is_none());
}

/// Applies a fixed scenario through the real per-tick code path and returns
/// the replay journal it wrote.
fn run_scenario(messages: Vec<ToGame>, tick: u64) -> String {
    let mut scheduler = Scheduler::new(vec![Region::new(SINGLE_REGION)]);
    let (sender, mut incoming) = mpsc::channel(64);
    for message in messages {
        sender.try_send(message).expect("the queue has room");
    }
    drop(sender);
    let mut replay = Replay::new(Some(Vec::new()));
    let mut disconnected = false;
    apply_queued(
        &mut scheduler,
        &mut replay,
        tick,
        &mut incoming,
        &mut disconnected,
    );
    assert!(disconnected, "the scenario drains and then sees the end");
    let bytes = replay.into_sink().expect("the sink was enabled");
    String::from_utf8(bytes).expect("the journal is UTF-8")
}

/// A fresh join → teleport_ack → quit scenario.
fn join_ack_quit() -> Vec<ToGame> {
    let (join, _receiver, _kick, _slot) = join_fixture_with(1, OUTBOUND_CAPACITY);
    vec![
        ToGame::Join(Box::new(join)),
        ToGame::TeleportAck {
            id: 1,
            teleport_id: 1,
        },
        ToGame::Quit { id: 1 },
    ]
}

#[test]
fn the_same_scenario_is_recorded_twice_identically() {
    let first = run_scenario(join_ack_quit(), 3);
    let second = run_scenario(join_ack_quit(), 3);
    assert_eq!(
        first, second,
        "the same scenario must record the same bytes"
    );

    let lines: Vec<&str> = first.lines().collect();
    assert_eq!(lines.len(), 3, "one line per applied event");

    let join: serde_json::Value = serde_json::from_str(lines[0]).expect("a JSON line");
    assert_eq!(join["tick"], 3);
    assert_eq!(join["seq"], 0);
    assert_eq!(join["kind"], "join");
    assert_eq!(join["src"], 1);
    assert_eq!(join["region"], SINGLE_REGION);
    assert_eq!(join["payload"]["name"], "player-1");

    let ack: serde_json::Value = serde_json::from_str(lines[1]).expect("a JSON line");
    assert_eq!(ack["seq"], 1, "order grows inside the tick");
    assert_eq!(ack["kind"], "teleport_ack");
    assert_eq!(ack["payload"]["teleport_id"], 1);

    let quit: serde_json::Value = serde_json::from_str(lines[2]).expect("a JSON line");
    assert_eq!(quit["seq"], 2);
    assert_eq!(quit["kind"], "quit");
    assert_eq!(quit["region"], SINGLE_REGION, "the player was still known");
}

/// Applies the queued messages of one tick, then one movement, ticks once and
/// returns the journal bytes — the same recording path the live loop uses.
fn run_movement_scenario(
    messages: Vec<ToGame>,
    slot: &Arc<MoveSlot>,
    movement: MoveState,
    tick: u64,
) -> String {
    let mut scheduler = Scheduler::new(vec![Region::new(SINGLE_REGION)]);
    let (sender, mut incoming) = mpsc::channel(64);
    for message in messages {
        sender.try_send(message).expect("the queue has room");
    }
    drop(sender);
    let mut replay = Replay::new(Some(Vec::new()));
    let mut disconnected = false;
    apply_queued(
        &mut scheduler,
        &mut replay,
        tick,
        &mut incoming,
        &mut disconnected,
    );
    slot.store(movement);
    let outcome = scheduler.tick(tick, Instant::now());
    record_moves(&mut replay, tick, &outcome.moves);
    let bytes = replay.into_sink().expect("the sink was enabled");
    String::from_utf8(bytes).expect("the journal is UTF-8")
}

#[test]
fn the_same_movement_scenario_is_recorded_twice_identically() {
    // A fresh connection per run: the slot is per player, not shared.
    let scenario = || {
        let (join, _receiver, _kick, slot) = join_fixture_with(1, OUTBOUND_CAPACITY);
        let messages = vec![
            ToGame::Join(Box::new(join)),
            ToGame::TeleportAck {
                id: 1,
                teleport_id: 1,
            },
        ];
        run_movement_scenario(messages, &slot, pos_move(1.5, 64.0, -2.5), 4)
    };

    let first = scenario();
    let second = scenario();
    assert_eq!(
        first, second,
        "the same movement must record the same bytes"
    );

    let lines: Vec<&str> = first.lines().collect();
    assert_eq!(lines.len(), 3, "join, teleport_ack and one move");
    let line: serde_json::Value = serde_json::from_str(lines[2]).expect("a JSON line");
    assert_eq!(line["kind"], "move");
    assert_eq!(line["tick"], 4);
    assert_eq!(line["seq"], 2, "the move is recorded after the messages");
    assert_eq!(line["region"], SINGLE_REGION);
    assert_eq!(line["src"], 1);
    assert_eq!(line["payload"]["x"], 1.5);
    assert_eq!(line["payload"]["y"], 64.0);
    assert_eq!(line["payload"]["z"], -2.5);
    assert_eq!(line["payload"]["on_ground"], true);
    assert_eq!(line["payload"]["flags"], 1);
}

#[test]
fn a_disabled_journal_records_nothing() {
    let mut replay: Replay<Vec<u8>> = Replay::new(None);
    assert!(!replay.enabled());
    replay.start_tick();
    replay.record(0, Some(SINGLE_REGION), 1, "join", serde_json::json!({}));
    assert!(replay.into_sink().is_none(), "no sink, no journal");
}

#[test]
fn the_loop_runs_the_configured_number_of_ticks() {
    let config = GameConfig {
        period: Duration::from_millis(1),
        ticks_limit: Some(20),
        ..GameConfig::default()
    };
    let (thread, handle) = GameHandle::spawn(config).expect("spawn the game thread");
    thread.join().expect("the game thread panicked");
    let metrics = handle.metrics();
    assert_eq!(metrics.ticks, 20, "the loop must stop at the limit");
    assert!(
        metrics.duration_p99_ms < 50.0,
        "a 1 ms tick must stay short, got {} ms",
        metrics.duration_p99_ms
    );
}

#[test]
fn player_ids_start_at_one_and_never_repeat() {
    let config = GameConfig {
        period: Duration::from_millis(1),
        ticks_limit: Some(1),
        ..GameConfig::default()
    };
    let (thread, handle) = GameHandle::spawn(config).expect("spawn the game thread");
    assert_eq!(handle.next_player_id(), 1, "the first id is the join's");
    assert_eq!(handle.next_player_id(), 2, "ids never repeat");
    thread.join().expect("the game thread panicked");
}

#[test]
fn the_configured_game_mode_reaches_the_handle() {
    // M4.5 Шаг 7.3: the joining player's game mode comes from the server's
    // `default_game_mode`; the default is survival (0), as before.
    let config = GameConfig {
        period: Duration::from_millis(1),
        ticks_limit: Some(1),
        default_game_mode: 3,
        ..GameConfig::default()
    };
    let (thread, handle) = GameHandle::spawn(config).expect("spawn the game thread");
    assert_eq!(handle.default_game_mode(), 3, "the configured mode");
    thread.join().expect("the game thread panicked");

    let (_thread, handle) =
        GameHandle::spawn(GameConfig::default()).expect("spawn the game thread");
    assert_eq!(handle.default_game_mode(), 0, "survival is the default");
}

#[test]
fn a_game_mode_outside_zero_to_three_is_rejected_at_startup() {
    for mode in [-1, 4, 255] {
        let config = GameConfig {
            default_game_mode: mode,
            ..GameConfig::default()
        };
        let error = GameHandle::spawn(config)
            .err()
            .unwrap_or_else(|| panic!("game mode {mode} must be rejected"));
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::InvalidInput,
            "game mode {mode}"
        );
    }
}

#[test]
fn the_thread_stops_on_request_within_the_timeout() {
    // No tick limit: on its own this loop would run forever.
    let (thread, handle) = GameHandle::spawn(GameConfig::default()).expect("spawn the game thread");
    std::thread::sleep(Duration::from_millis(120));

    let started = Instant::now();
    thread
        .join_timeout(Duration::from_secs(2))
        .expect("the loop stopped in time");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "the stop must be seen within a tick or two, took {elapsed:?}"
    );
    assert!(
        handle.metrics().ticks >= 1,
        "the loop had been ticking before the stop"
    );
}

#[test]
fn join_timeout_reports_a_thread_that_did_not_stop() {
    let (thread, _handle) =
        GameHandle::spawn(GameConfig::default()).expect("spawn the game thread");
    // A fresh loop cannot have left before its first 50 ms deadline.
    let err = thread
        .join_timeout(Duration::from_millis(0))
        .expect_err("a fresh loop is still running");
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
}

/// The M4.1 acceptance test: 1000 idle ticks of the real 20 TPS deadline.
///
/// It runs ~50 s of wall clock, so it is ignored by default (D6: CI runs
/// only the smoke checks); run it with `--ignored --nocapture` and read the
/// numbers off the log line.
#[test]
#[ignore = "runs 1000 real 50 ms ticks (~50 s); run with --ignored --nocapture"]
fn a_thousand_idle_ticks_do_not_drift() {
    let config = GameConfig {
        ticks_limit: Some(1000),
        ..GameConfig::default()
    };
    let (thread, handle) = GameHandle::spawn(config).expect("spawn the game thread");
    thread.join().expect("the game thread panicked");
    let m = handle.metrics();
    println!(
        "1000 idle ticks: drift {:.1} ms | jitter p50 {:.2} p99 {:.2} max {:.2} ms | \
         duration p50 {:.3} p99 {:.3} max {:.3} ms",
        m.drift_ms,
        m.jitter_p50_ms,
        m.jitter_p99_ms,
        m.jitter_max_ms,
        m.duration_p50_ms,
        m.duration_p99_ms,
        m.duration_max_ms
    );
    assert_eq!(m.ticks, 1000, "must have run exactly 1000 ticks");
    assert!(
        m.drift_ms.abs() < 5.0,
        "drift {} ms must be under 5 ms",
        m.drift_ms
    );
    assert!(
        m.jitter_p99_ms < 20.0,
        "jitter p99 {} ms must be under 20 ms",
        m.jitter_p99_ms
    );
}

/// Three confirmed players of region `region`, each with one queued move.
///
/// The same fixture builds every scheduler of the pool tests, so a different
/// `K` sees byte-identical input and only the transport may differ.
fn pool_scenario() -> Vec<Region> {
    let mut regions = Vec::new();
    for region in 0..8 {
        let mut built = Region::new(region);
        for player in 0..2 {
            let id = region * 10 + player + 1;
            let (join, _receiver, _kick, slot) = join_fixture_with(id, OUTBOUND_CAPACITY);
            built.insert(join);
            if let Some(player) = built.get_mut(id) {
                player.confirm_teleport();
            }
            slot.store(pos_move(
                f64::from(id),
                64.0 + f64::from(player),
                -f64::from(id),
            ));
        }
        regions.push(built);
    }
    regions
}

#[test]
fn k_zero_one_and_four_merge_the_same_outcome_for_eight_regions() {
    let mut inline = Scheduler::with_workers(pool_scenario(), 0);
    assert_eq!(inline.worker_count(), 0, "K=0 keeps the inline tick");
    let expected = inline.tick(7, Instant::now());
    assert_eq!(expected.moves.len(), 16, "two moves per region");

    for k in [1usize, 4] {
        let mut scheduler = Scheduler::with_workers(pool_scenario(), k);
        assert_eq!(scheduler.worker_count(), k, "K={k} starts {k} workers");
        let pooled = scheduler.tick(7, Instant::now());
        assert_eq!(
            pooled, expected,
            "K={k} must merge exactly the inline result"
        );
        assert_eq!(scheduler.regions().len(), 8, "every region came back");
        assert_eq!(scheduler.player_count(), 16, "no player was lost");
    }
}

#[test]
fn the_merge_order_does_not_depend_on_the_completion_order() {
    let mut base = Scheduler::with_workers(pool_scenario(), 4);
    let baseline = base.tick(1, Instant::now());

    let mut regions = pool_scenario();
    // Region 0 finishes ~50 ms after the others; the merge must still be
    // ordered by region id, not by who returned first.
    regions[0].set_test_delay(Duration::from_millis(50));
    let mut delayed = Scheduler::with_workers(regions, 4);
    let outcome = delayed.tick(1, Instant::now());
    assert_eq!(outcome, baseline, "a late region changes no order");
    let ids: Vec<i32> = outcome.moves.iter().map(|moved| moved.region).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "moves are merged in region id order");
}

#[test]
fn the_region_pool_stops_within_two_seconds() {
    let scheduler = Scheduler::with_workers(vec![Region::new(SINGLE_REGION)], 4);
    assert_eq!(scheduler.worker_count(), 4);
    let started = Instant::now();
    drop(scheduler);
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "the pool took {:?} to stop",
        started.elapsed()
    );
}

/// The game loop with a pool: the tick still runs and the pool joins on exit.
#[test]
fn the_loop_runs_and_stops_with_a_region_pool() {
    let config = GameConfig {
        period: Duration::from_millis(1),
        ticks_limit: Some(5),
        workers: 4,
        ..GameConfig::default()
    };
    let (thread, handle) = GameHandle::spawn(config).expect("spawn the game thread");
    thread.join().expect("the game thread panicked");
    assert_eq!(handle.metrics().ticks, 5, "the loop stopped at the limit");
}

/// The M4.3 A2 acceptance test: 1000 idle 20 TPS ticks through the pool.
///
/// Four empty regions exercising the dispatch/barrier path (a single region
/// would tick inline by design). ~50 s of wall clock, so it is ignored like
/// the M4.1 measurement; run it with `--ignored --nocapture`.
#[test]
#[ignore = "runs 1000 real 50 ms pooled ticks (~50 s); run with --ignored --nocapture"]
fn a_thousand_idle_pooled_ticks_do_not_drift() {
    let period = TICK_PERIOD;
    let regions: Vec<Region> = (0..4).map(Region::new).collect();
    let mut scheduler = Scheduler::with_workers(regions, 4);
    assert_eq!(scheduler.worker_count(), 4);
    let start = Instant::now();
    let mut deadline = start + period;
    let mut jitter: Vec<u64> = Vec::with_capacity(1000);
    for tick in 0..1000u64 {
        let now = Instant::now();
        if deadline > now {
            std::thread::sleep(deadline - now);
        }
        let woke = Instant::now();
        jitter.push(woke.saturating_duration_since(deadline).as_micros() as u64);
        let _ = scheduler.tick(tick, deadline);
        deadline += period;
    }
    let expected = period.mul_f64(1000.0);
    let drift_ms = (start.elapsed().as_secs_f64() - expected.as_secs_f64()) * 1000.0;
    jitter.sort_unstable();
    let p50 = jitter[jitter.len() / 2] as f64 / 1000.0;
    let p99 = jitter[jitter.len() * 99 / 100] as f64 / 1000.0;
    let max = *jitter.last().expect("1000 samples") as f64 / 1000.0;
    println!(
        "1000 idle pooled ticks (K=4, 4 regions): drift {drift_ms:.3} ms | \
         jitter p50 {p50:.3} p99 {p99:.3} max {max:.3} ms"
    );
    assert!(
        drift_ms.abs() < 5.0,
        "drift {drift_ms} ms must be under 5 ms"
    );
    assert!(p99 < 20.0, "jitter p99 {p99} ms must be under 20 ms");
}

/// Measurement only: one region, inline tick against forced dispatch.
///
/// Prints p50/p99 of `Scheduler::tick` in microseconds; the decision it backs
/// is in `docs/DECISIONS.md` (inline for ≤1 region, hypothesis until M4.6).
#[test]
#[ignore = "measurement; run with --ignored --nocapture (--release for numbers)"]
fn inline_versus_pool_dispatch_on_one_region() {
    let (inline_p50, inline_p99) = measure_one_region(false);
    let (dispatch_p50, dispatch_p99) = measure_one_region(true);
    println!(
        "one region, 20000 ticks: inline p50 {inline_p50:.3} p99 {inline_p99:.3} us | \
         pool dispatch p50 {dispatch_p50:.3} p99 {dispatch_p99:.3} us"
    );
}

/// Times `samples` ticks of a single region, inline or through the pool.
///
/// Nanosecond resolution: a single dispatch is a channel round trip of a few
/// microseconds at most, invisible to a millisecond clock.
fn measure_one_region(dispatch: bool) -> (f64, f64) {
    const SAMPLES: usize = 20_000;
    const WARMUP: usize = 1_000;
    let mut scheduler = Scheduler::with_workers(vec![Region::new(SINGLE_REGION)], 4);
    if dispatch {
        scheduler.force_dispatch();
    }
    let now = Instant::now();
    let mut durations: Vec<u64> = Vec::with_capacity(SAMPLES);
    for tick in 0..(WARMUP + SAMPLES) {
        let started = Instant::now();
        let _ = scheduler.tick(tick as u64, now);
        let took = started.elapsed().as_nanos() as u64;
        if tick >= WARMUP {
            durations.push(took);
        }
    }
    durations.sort_unstable();
    let p50 = durations[durations.len() / 2] as f64 / 1000.0;
    let p99 = durations[durations.len() * 99 / 100] as f64 / 1000.0;
    (p50, p99)
}
