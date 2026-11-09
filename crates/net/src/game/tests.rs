//! Tests of the game thread's timing and bookkeeping.

use std::sync::Arc;
use std::time::{Duration, Instant};

use protocol::Position;
use tokio::sync::mpsc;

use super::*;

/// A join registration with a queue of the given capacity, the connection's
/// side of that queue, and the player's kick flag.
fn join_fixture_with(
    id: PlayerId,
    capacity: usize,
) -> (PlayerJoin, mpsc::Receiver<ServerMessage>, Arc<KickFlag>) {
    let (outbound, receiver) = mpsc::channel(capacity.max(1));
    let kick = Arc::new(KickFlag::default());
    let join = PlayerJoin {
        id,
        name: format!("player-{id}"),
        view_distance: 10,
        spawn: Position::default(),
        connection: PlayerConnection {
            outbound,
            kick: Arc::clone(&kick),
            last_move: Arc::new(MoveSlot::default()),
        },
    };
    (join, receiver, kick)
}

/// A join registration plus the connection's side of its outbound queue.
fn join_fixture(id: PlayerId) -> (PlayerJoin, mpsc::Receiver<ServerMessage>) {
    let (join, receiver, _kick) = join_fixture_with(id, OUTBOUND_CAPACITY);
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
        packet_id: 0x1E,
        payload: vec![1, 2, 3],
    });
    slot.store(MoveState {
        packet_id: 0x1F,
        payload: vec![4, 5, 6],
    });
    let newest = slot.take().expect("the newest packet");
    assert_eq!(newest.packet_id, 0x1F, "the older packet is overwritten");
    assert!(slot.take().is_none(), "taking drains the slot");
}

#[test]
fn a_full_outbound_queue_kicks_the_player_and_frees_the_region() {
    let mut region = Region::new(SINGLE_REGION);
    let (join, _receiver, kick) = join_fixture_with(7, 1);
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
    let (join, receiver, kick) = join_fixture_with(9, 1);
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
    let (join, _receiver, _kick) = join_fixture_with(1, OUTBOUND_CAPACITY);
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
