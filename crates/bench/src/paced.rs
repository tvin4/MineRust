//! The paced engine: the real 20 TPS absolute-deadline loop (ARCH-M4 §3/§5).
//!
//! Measures, over `seconds` of wall time (3 repeats are the caller's job):
//! drift, jitter p50/p99/p999 (wake latencies), tick duration
//! p50/p99/p999/max, late ticks (wake > 20 ms) and missed slots (ticks that
//! started after the next deadline had already passed).

use std::time::{Duration, Instant};

use net::game::{RegionGrid, Scheduler, TICK_PERIOD};

use crate::players::{register_all, set_step, tick_moves, BenchPlayer};
use crate::report::percentiles;
use crate::{BenchSpec, Workload};

/// Wake latency over which a tick counts as "late" — the game thread's own
/// threshold (`metrics::LATE_TICK_MS`), kept identical here so the numbers
/// are comparable.
const LATE_TICK_MS: u128 = 20;

/// One paced run's measured facts.
#[derive(Debug, Clone)]
pub struct PacedResult {
    /// Jitter (wake latencies) in µs: p50, p99, p999.
    pub jitter_us: [f64; 3],
    /// Tick duration in µs: p50, p99, p999, max.
    pub tick_us: [f64; 4],
    /// How far behind the schedule the loop finished, in µs (positive =
    /// behind; the drift ARCH-M4 §3 measures).
    pub drift_us: f64,
    /// Ticks that woke more than 20 ms after their deadline.
    pub late_ticks: u64,
    /// Ticks that started after the following deadline had already passed
    /// (the loop could not keep up: those slots were missed).
    pub missed_slots: u64,
    /// Total wall time the run took.
    pub elapsed: Duration,
}

/// Builds the scheduler exactly like the capacity engine does.
pub fn build(spec: BenchSpec, players: &mut [BenchPlayer]) -> anyhow::Result<Scheduler> {
    let mut scheduler =
        Scheduler::with_grid(Vec::new(), RegionGrid::new(spec.region_side), spec.workers);
    register_all(&mut scheduler, players);
    if spec.workload != Workload::W1 {
        let ids: Vec<i32> = scheduler.regions().iter().map(|r| r.id()).collect();
        for id in ids {
            if let Some(region) = scheduler.region_mut(id) {
                region.set_bench_workload(spec.workload.duration());
            }
        }
    }
    Ok(scheduler)
}

/// Runs one paced measurement: absolute-deadline loop at 20 TPS for
/// `spec.seconds` of measured ticks, `spec.warmup` warm-up ticks discarded.
pub fn run(
    mut scheduler: Scheduler,
    players: &mut [BenchPlayer],
    spec: BenchSpec,
) -> anyhow::Result<PacedResult> {
    let started = Instant::now();
    let mut deadline = started + TICK_PERIOD;
    let mut jitter: Vec<f64> = Vec::new();
    let mut ticks: Vec<f64> = Vec::new();
    let (mut late, mut missed, mut counted) = (0u64, 0u64, 0u64);
    let mut tick: u64 = 0;
    let measured_ticks = spec.seconds * 20;
    while counted < measured_ticks {
        let now = Instant::now();
        if now < deadline {
            std::thread::sleep(deadline - now);
        }
        let woke = Instant::now();
        let late_by = woke.saturating_duration_since(deadline);
        if tick >= spec.warmup {
            jitter.push(late_by.as_secs_f64() * 1e6);
            if late_by.as_millis() > LATE_TICK_MS {
                late += 1;
            }
            if woke > deadline + TICK_PERIOD {
                missed += 1;
            }
        }
        let work_started = Instant::now();
        let outcome = scheduler.tick(tick, deadline);
        drop(outcome);
        tick_moves(players);
        if tick >= spec.warmup {
            ticks.push(work_started.elapsed().as_secs_f64() * 1e6);
            counted += 1;
        }
        tick += 1;
        deadline += TICK_PERIOD;
    }
    let elapsed = started.elapsed();
    if jitter.is_empty() || ticks.is_empty() {
        anyhow::bail!("no measured ticks in {} s of pacing", spec.seconds);
    }
    // Drift: how far behind the loop finished relative to the schedule it
    // started with (positive µs = behind).
    let drift_us = (Instant::now() - deadline).as_secs_f64() * 1e6;
    Ok(PacedResult {
        jitter_us: {
            let p = percentiles(&jitter);
            [p[0], p[1], p[2]]
        },
        tick_us: percentiles(&ticks),
        drift_us,
        late_ticks: late,
        missed_slots: missed,
        elapsed,
    })
}

/// One full paced run: build, measure, return the facts. Players walk 0.25
/// blocks/tick (+x) so the tick decodes and applies real moves (W1).
pub fn measure(spec: BenchSpec, spread_chunks: i32) -> anyhow::Result<PacedResult> {
    let mut players = crate::players::spread(spec.players, spread_chunks, (-64, -64));
    set_step(&mut players, 0.25);
    let scheduler = build(spec, &mut players)?;
    run(scheduler, &mut players, spec)
}

/// Runs the CLI mode: one default run; the full matrix is manual
/// (docs/BENCH-M4.md records the series).
pub fn run_cli(label: Option<&str>) -> anyhow::Result<()> {
    let spec = BenchSpec::default();
    let result = measure(spec, 8)?;
    let label = label.unwrap_or("default");
    let line = format!(
        "| {label} | {} | {} | {} | {} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.1} |",
        spec.workers,
        spec.region_side,
        spec.players,
        spec.workload.label(),
        result.jitter_us[0],
        result.jitter_us[1],
        result.jitter_us[2],
        result.tick_us[0],
        result.tick_us[1],
        result.drift_us / 1000.0,
        result.late_ticks,
        result.missed_slots,
        result.elapsed.as_secs_f64(),
    );
    let path = crate::report::write_report(
        "paced",
        label,
        &format!(
            "# paced: {label}\n\n\
             all times µs; jitter = wake latency past the absolute deadline\n\n\
             | run | K | side | P | W | jitter p50 | jitter p99 | jitter p999 | tick p50 | tick p99 | drift ms | late | missed | elapsed s |\n\
             |---|---|---|---|---|---|---|---|---|---|---|---|---|---|\n{line}\n"
        ),
    )?;
    println!("{line}\nreport: {}", path.display());
    Ok(())
}
