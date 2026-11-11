//! The capacity engine: the scheduler called in a loop with no tick-wait
//! pause (ARCH-M4 §5). Measures tick duration (p50/p99/p999/max) and
//! throughput (ticks/s) over a fixed tick count, warm-up discarded.
//!
//! Outcomes (`moves`, `transfers`, `streams`) are dropped immediately so the
//! outbound queues behind every player never fill: the measurement is of the
//! tick mechanism, not of backpressure nobody is reading.

use std::time::Instant;

use net::game::Scheduler;

use crate::players::{register_all, set_step, tick_moves, BenchPlayer};
use crate::report::percentiles;
use crate::{BenchSpec, Workload};

/// One run's measured facts.
#[derive(Debug, Clone)]
pub struct CapacityResult {
    /// Tick duration percentiles in µs: p50, p99, p999, max.
    pub tick_us: [f64; 4],
    /// Ticks per second over the measured span (work time only, no pauses:
    /// the capacity mode never waits for a deadline).
    pub tps: f64,
}

/// Builds the scheduler for `spec`: players registered over the grid, W2
/// workload set on every region if any.
pub fn build(spec: BenchSpec, players: &mut [BenchPlayer]) -> anyhow::Result<Scheduler> {
    let mut scheduler = Scheduler::with_grid(
        Vec::new(),
        net::game::RegionGrid::new(spec.region_side),
        spec.workers,
    );
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

/// Runs one capacity measurement: `spec.warmup` discarded ticks, then
/// `spec.ticks` measured ones. Every tick each walking player stores one
/// move packet through its [`net::game::MoveSlot`].
pub fn run(
    mut scheduler: Scheduler,
    players: &mut [BenchPlayer],
    spec: BenchSpec,
) -> anyhow::Result<CapacityResult> {
    let mut samples = Vec::with_capacity(spec.ticks as usize);
    for tick in 0..spec.warmup + spec.ticks {
        tick_moves(players);
        let started = Instant::now();
        let outcome = scheduler.tick(tick, started);
        if tick >= spec.warmup {
            samples.push(started.elapsed().as_secs_f64() * 1e6);
        }
        drop(outcome);
    }

    if samples.is_empty() {
        anyhow::bail!(
            "no measured ticks: warmup ({}) ate the whole run",
            spec.warmup
        );
    }
    let busy: f64 = samples.iter().sum();
    let tps = samples.len() as f64 * 1e6 / busy;
    Ok(CapacityResult {
        tick_us: percentiles(&samples),
        tps,
    })
}

/// One full run: build, measure, return the facts. Players walk 0.25
/// blocks/tick (+x) so the tick decodes and applies real moves (W1).
pub fn measure(spec: BenchSpec, spread_chunks: i32) -> anyhow::Result<CapacityResult> {
    let mut players = crate::players::spread(spec.players, spread_chunks, (-64, -64));
    set_step(&mut players, 0.25);
    let scheduler = build(spec, &mut players)?;
    run(scheduler, &mut players, spec)
}

/// Parses the optional CLI spec: the label either carries a W2 workload
/// (`W2:<µs>...` — the scalability-ceiling series, the prefix must come
/// first) or stays W1. The pool shape stays the `BenchSpec` default; the
/// matrix series are manual.
fn parse_workload(label: &str) -> Workload {
    match label.strip_prefix("W2:") {
        Some(rest) => {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse().map_or(Workload::W1, Workload::W2)
        }
        None => Workload::W1,
    }
}

/// Runs the CLI mode: one default run; the full matrix is manual
/// (docs/BENCH-M4.md records the series).
pub fn run_cli(label: Option<&str>) -> anyhow::Result<()> {
    let label = label.unwrap_or("default");
    let spec = BenchSpec {
        workload: parse_workload(label),
        ..BenchSpec::default()
    };
    let result = measure(spec, 8)?;
    let line = crate::report::run_table(label, spec, result.tick_us, result.tps);
    // Windows filenames: no colons.
    let safe_label = label.replace(':', "-");
    let path = crate::report::write_report(
        "capacity",
        &safe_label,
        &format!(
            "# capacity: {label}\n\n| run | K | side | P | W | p50 µs | p99 µs | p999 µs | max µs | ticks/s |\n|---|---|---|---|---|---|---|---|---|---|\n{line}\n"
        ),
    )?;
    println!("{line}\nreport: {}", path.display());
    Ok(())
}
