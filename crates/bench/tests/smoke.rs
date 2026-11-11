//! The CI smoke test: one tiny capacity run plus one tiny paced run (~1 s
//! total), 2 bots, K=2, a synthetic two-region split — independent of the
//! host's core count and green on both ubuntu and windows (D6). Full runs
//! stay manual.

use std::time::{Duration, Instant};

use bench::players::{set_step, spread};
use bench::{BenchSpec, Workload};

/// A 2-bot, K=2, 2-region capacity run must complete and collect samples;
/// the wall time stays around a second.
#[test]
fn a_two_bot_smoke_capacity_run_completes() {
    let spec = BenchSpec {
        workers: 2,
        region_side: 16,
        players: 2,
        workload: Workload::W1,
        warmup: 20,
        ticks: 100,
        seconds: 0,
    };
    let started = Instant::now();
    let mut players = spread(spec.players, 8, (0, 0));
    set_step(&mut players, 0.25);
    let scheduler = bench::capacity::build(spec, &mut players).expect("scheduler builds");
    let result = bench::capacity::run(scheduler, &mut players, spec).expect("run measures");
    let wall = started.elapsed();
    assert!(result.tick_us.iter().all(|&v| v > 0.0), "samples collected");
    assert!(result.tps > 0.0, "throughput computed");
    assert!(
        wall < Duration::from_secs(5),
        "smoke stays tiny, took {wall:?}"
    );
}

/// The paced engine survives a second of real 20 TPS pacing too (the smoke
/// budget stays ~1 s total across both tests).
#[test]
fn a_two_bot_smoke_paced_run_keeps_its_deadlines() {
    let spec = BenchSpec {
        workers: 2,
        region_side: 16,
        players: 2,
        workload: Workload::W1,
        warmup: 5,
        ticks: 0,
        seconds: 1,
    };
    let mut players = spread(spec.players, 8, (0, 0));
    set_step(&mut players, 0.25);
    let scheduler = bench::paced::build(spec, &mut players).expect("scheduler builds");
    let result = bench::paced::run(scheduler, &mut players, spec).expect("run measures");
    assert!(
        result.late_ticks < 5,
        "mostly on time: {:?}",
        result.jitter_us
    );
    assert_eq!(
        result.missed_slots, 0,
        "a 2-bot W1 tick never misses a slot"
    );
}
