//! Tests of the game thread's timing and bookkeeping.

use std::sync::Arc;
use std::time::Duration;

use protocol::Position;
use tokio::sync::mpsc;

use super::*;

/// A join registration plus the connection's side of its outbound queue.
fn join_fixture(id: PlayerId) -> (PlayerJoin, mpsc::Receiver<ServerMessage>) {
    let (outbound, receiver) = mpsc::channel(OUTBOUND_CAPACITY);
    let join = PlayerJoin {
        id,
        name: format!("player-{id}"),
        view_distance: 10,
        spawn: Position::default(),
        connection: PlayerConnection {
            outbound,
            kick: Arc::new(KickFlag::default()),
            last_move: Arc::new(MoveSlot::default()),
        },
    };
    (join, receiver)
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
