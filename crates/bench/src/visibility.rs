//! The M4.5 Шаг 3.5 visibility measurement: the capacity engine run on two
//! player geometries with the entity-visibility pass on.
//!
//! No network: every bench player writes its move packet into a real
//! [`net::game::MoveSlot`] exactly like a connection would, the scheduler is
//! built with `region_side = 0` (one region, inline) and the tick's outcome is
//! dropped. The players register **before** the warm-up, so the join phase is
//! excluded and only steady walking at 0.25 blocks/tick is measured
//! (BENCH-M4 §5).
//!
//! Two geometries:
//!
//! * **dense** — a compact square grid one block apart, so every player is
//!   inside every other player's 160-block view radius: the visible set is all
//!   `P − 1` others. This is the worst case of the pass.
//! * **uniform** — spread over 1000×1000 blocks, so a player sees about
//!   `P · π · 160² / 1000²` neighbours — about 16 at `P = 200`.
//!
//! The stop rule of Шаг 3.5: if the steady-state p99 at (b) `P = 200` exceeds
//! 5 ms, the run says so and the sequence stops.

use std::time::Duration;

use net::game::{RegionGrid, Scheduler};

use crate::capacity::{self, CapacityResult};
use crate::players::{self, register_all, set_step};

/// Which Шаг 3.5 geometry to measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Geometry {
    /// A compact cluster, all players mutually visible.
    Dense,
    /// A uniform spread over 1000×1000 blocks.
    Uniform,
}

impl Geometry {
    /// Stable label for the report.
    pub fn label(self) -> &'static str {
        match self {
            Self::Dense => "dense",
            Self::Uniform => "uniform",
        }
    }
}

/// Side of the uniform geometry, in blocks.
pub const UNIFORM_SPAN: i32 = 1000;

/// The steady-state p99 above which the uniform `P = 200` run stops the
/// sequence, in microseconds (5 ms).
pub const STOP_P99_US: f64 = 5000.0;

/// Builds the players of a geometry and measures one capacity run: the
/// players are registered, their teleports confirmed and they walk 0.25
/// blocks/tick; the join is inside the discarded warm-up.
pub fn measure(
    geometry: Geometry,
    count: usize,
    warmup: Duration,
    measure: Duration,
) -> anyhow::Result<CapacityResult> {
    let mut players = match geometry {
        Geometry::Dense => players::dense_cluster(count),
        Geometry::Uniform => players::uniform_area(count, UNIFORM_SPAN),
    };
    set_step(&mut players, 0.25);
    // One region, inline: the geometry places the players, the visibility
    // pass is what is measured, not the pool (W2 covers the pool separately).
    let mut scheduler = Scheduler::with_grid(Vec::new(), RegionGrid::new(0), 0);
    register_all(&mut scheduler, &mut players);
    capacity::run_timed(scheduler, &mut players, warmup, measure)
}

/// Runs the Шаг 3.5 series: dense and uniform, `P` of 64/200/512, three
/// repeats each, and writes the report. Returns the markdown.
///
/// A steady p99 above [`STOP_P99_US`] on the uniform `P = 200` runs makes the
/// function fail after the report is written: the Шаг 3.5 stop rule.
pub fn run_cli() -> anyhow::Result<String> {
    let warmup = Duration::from_secs(3);
    let window = Duration::from_secs(15);
    let counts = [64usize, 200, 512];
    let repeats = 3;
    let mut markdown = String::from(
        "# M4.5 Шаг 3.5 — entity-visibility pass, capacity engine (BENCH-M4 §5)\n\n\
         No network: one region (`side = 0`), inline. Players register before \
         the run (join in the discarded warm-up) and walk 0.25 blocks/tick. \
         dense = a compact cluster, all players mutually visible; uniform = \
         1000×1000 blocks, ~16 visible neighbours at P=200. Durations in ms.\n\n\
         | geometry | P | repeat | p50 ms | p99 ms | max ms | ticks/s |\n\
         |---|---|---|---|---|---|---|\n",
    );
    let mut stopped = Vec::new();
    for geometry in [Geometry::Dense, Geometry::Uniform] {
        for &count in &counts {
            for repeat in 1..=repeats {
                let result = measure(geometry, count, warmup, window)?;
                let [p50, p99, _p999, max] = result.tick_us;
                markdown.push_str(&format!(
                    "| {} | {} | {} | {:.3} | {:.3} | {:.3} | {:.0} |\n",
                    geometry.label(),
                    count,
                    repeat,
                    p50 / 1000.0,
                    p99 / 1000.0,
                    max / 1000.0,
                    result.tps,
                ));
                println!(
                    "{} P={count} repeat {repeat}: p50/p99/max = {:.3}/{:.3}/{:.3} ms, {:.0} ticks/s",
                    geometry.label(),
                    p50 / 1000.0,
                    p99 / 1000.0,
                    max / 1000.0,
                    result.tps,
                );
                if geometry == Geometry::Uniform && count == 200 && p99 > STOP_P99_US {
                    stopped.push(format!(
                        "uniform P=200 repeat {repeat}: p99 {:.3} ms > 5 ms",
                        p99 / 1000.0
                    ));
                }
            }
        }
    }
    let path = crate::network::save_report("m45-visibility", &markdown)?;
    println!("report: {}", path.display());
    if !stopped.is_empty() {
        for line in &stopped {
            println!("STOP: {line}");
        }
        anyhow::bail!(
            "Шаг 3.5 stop rule: uniform P=200 steady p99 above 5 ms ({}); report: {}",
            stopped.join("; "),
            path.display()
        );
    }
    Ok(markdown)
}
