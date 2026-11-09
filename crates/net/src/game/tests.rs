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

/// Payload of `move_player_pos`: three big-endian doubles and the flag.
fn pos_payload(x: f64, y: f64, z: f64, on_ground: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(25);
    for value in [x, y, z] {
        out.extend_from_slice(&value.to_be_bytes());
    }
    out.push(u8::from(on_ground));
    out
}

/// A `move_player_pos` slot value.
fn pos_move(x: f64, y: f64, z: f64) -> MoveState {
    MoveState {
        kind: MoveKind::Pos,
        payload: pos_payload(x, y, z, true),
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
    let outcome = region.tick(0, Instant::now());

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
    let outcome = region.tick(7, Instant::now());

    assert_eq!(outcome.moves.len(), 1, "the move is applied once");
    let applied = outcome.moves[0];
    assert_eq!(applied.region, SINGLE_REGION);
    assert_eq!(applied.id, 1);
    assert_eq!(
        (applied.position.x, applied.position.y, applied.position.z),
        (100.5, 70.0, -50.25)
    );
    assert!(applied.position.on_ground);

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
    payload.push(0);
    slot.store(MoveState {
        kind: MoveKind::Rot,
        payload,
    });

    let outcome = region.tick(0, Instant::now());
    let after = outcome.moves[0].position;
    assert_eq!(
        (after.x, after.y, after.z),
        (before.x, before.y, before.z),
        "rotation does not move the player"
    );
    assert_eq!(after.yaw, 90.0);
    assert_eq!(after.pitch, -45.0);
    assert!(!after.on_ground);
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
