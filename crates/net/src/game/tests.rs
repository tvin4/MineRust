//! Tests of the game thread's timing and bookkeeping.

use std::time::Duration;

use super::*;

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
