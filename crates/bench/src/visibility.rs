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

use std::time::{Duration, Instant};

use net::entity_packets::push_entity_stream;
use net::game::{RegionGrid, Scheduler, ServerMessage};
use protocol::PacketKey;

use crate::capacity::{self, CapacityResult};
use crate::players::{self, register_all, set_step, tick_moves};

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

/// Per-phase percentiles of one phase-timing run, in μs (p50, p99, p999,
/// max). The phases are the ones of the Шаг 3.5 instrumentation:
///
/// * `visibility` — (а) the pair pass and the visible/not decision
///   ([`Scheduler::refresh_entities`]);
/// * `build` — (б) the packets the connections would build for the observers
///   (`add_entity`/`move_entity_*`/`rotate_head`/`player_info`);
/// * `deliver` — (в) the `try_send` of the entity messages into the
///   connections' outbound queues;
/// * `rest` — (г) the rest of the tick: applied moves, `StreamChunks` and
///   their delivery, transfers ([`Scheduler::tick_core`] plus the stream
///   delivery);
/// * `total` — the whole tick, from `tick_core` to the last delivery.
#[derive(Debug, Clone)]
pub struct PhaseResult {
    /// (а) visibility pass.
    pub visibility_us: [f64; 4],
    /// (б) packet build for the observers.
    pub build_us: [f64; 4],
    /// (в) `try_send` of the entity messages.
    pub deliver_us: [f64; 4],
    /// (г) the rest of the tick.
    pub rest_us: [f64; 4],
    /// Whole tick.
    pub total_us: [f64; 4],
    /// Ticks per second from the whole-tick samples.
    pub tps: f64,
}

/// Measures one phase-timing run: the same geometry, players and region setup
/// as [`measure`], but every tick is split into the four Шаг 3.5 phases.
///
/// The outbound queues are drained at the start of every tick, so phase (в)
/// meets a live (never full) queue exactly like a connection that keeps up;
/// a dropped receiver would make every `try_send` fail with `Closed` at once
/// and hide the channel cost.
pub fn measure_phases(
    geometry: Geometry,
    count: usize,
    warmup: Duration,
    measure: Duration,
) -> anyhow::Result<PhaseResult> {
    let mut players = match geometry {
        Geometry::Dense => players::dense_cluster(count),
        Geometry::Uniform => players::uniform_area(count, UNIFORM_SPAN),
    };
    set_step(&mut players, 0.25);
    let mut scheduler = Scheduler::with_grid(Vec::new(), RegionGrid::new(0), 0);
    register_all(&mut scheduler, &mut players);

    let mut visibility = Vec::new();
    let mut build = Vec::new();
    let mut deliver = Vec::new();
    let mut rest = Vec::new();
    let mut total = Vec::new();
    let mut packets: Vec<(PacketKey, Vec<u8>)> = Vec::new();
    let start = Instant::now();
    let mut tick: u64 = 0;
    loop {
        // Prepare this tick's move packets and empty every outbound queue.
        tick_moves(&mut players);
        for player in players.iter_mut() {
            player.drain_outbound();
        }

        // (г) the rest of the tick: moves, transfers, StreamChunks and their
        // delivery.
        let t0 = Instant::now();
        let outcome = scheduler.tick_core(tick, t0);
        for stream in &outcome.streams {
            let message = ServerMessage::StreamChunks {
                center: stream.center,
                add: stream.add.clone(),
                remove: stream.remove.clone(),
            };
            scheduler.deliver(stream.player, message);
        }
        let t1 = Instant::now();

        // (а) the visibility pass.
        let events = scheduler.refresh_entities();
        let t2 = Instant::now();

        // (б) the packets the connections would build, in send order.
        packets.clear();
        for event in &events {
            push_entity_stream(&event.spawn, &event.remove, &event.moves, &mut packets)?;
        }
        let t3 = Instant::now();

        // (в) the try_send of the entity messages.
        for event in events {
            let message = ServerMessage::StreamEntities {
                spawn: event.spawn,
                remove: event.remove,
                moves: event.moves,
            };
            scheduler.deliver(event.player, message);
        }
        let t4 = Instant::now();

        let elapsed = start.elapsed();
        if elapsed >= warmup {
            rest.push(t1.duration_since(t0).as_secs_f64() * 1e6);
            visibility.push(t2.duration_since(t1).as_secs_f64() * 1e6);
            build.push(t3.duration_since(t2).as_secs_f64() * 1e6);
            deliver.push(t4.duration_since(t3).as_secs_f64() * 1e6);
            total.push(t4.duration_since(t0).as_secs_f64() * 1e6);
        }
        if elapsed >= warmup + measure {
            break;
        }
        tick += 1;
    }
    if total.is_empty() {
        anyhow::bail!(
            "no measured ticks in {} s of warm-up + {} s window",
            warmup.as_secs(),
            measure.as_secs()
        );
    }
    let busy: f64 = total.iter().sum();
    let tps = total.len() as f64 * 1e6 / busy;
    Ok(PhaseResult {
        visibility_us: crate::report::percentiles(&visibility),
        build_us: crate::report::percentiles(&build),
        deliver_us: crate::report::percentiles(&deliver),
        rest_us: crate::report::percentiles(&rest),
        total_us: crate::report::percentiles(&total),
        tps,
    })
}

/// One measured run of the phase series.
struct PhaseRun {
    geometry: Geometry,
    count: usize,
    repeat: usize,
    result: PhaseResult,
}

/// Index into a percentile tuple: p50.
const P50: usize = 0;
/// Index into a percentile tuple: p99.
const P99: usize = 1;
/// Index into a percentile tuple: max.
const MAX: usize = 3;

/// One phase's value at a percentile, in ms.
fn ms(phase: &[f64; 4], which: usize) -> f64 {
    phase[which] / 1000.0
}

/// Renders the phase tables (p50/p99/max) and the dominance conclusion.
fn render_phases(runs: &[PhaseRun]) -> String {
    let mut out = String::from(
        "# M4.5 Шаг 3.5 — tick phases (BENCH-M4 §5)\n\n\
         One region (`side = 0`), inline, no network. Players register before \
         the run and walk 0.25 blocks/tick. The tick is split into four \
         phases; all durations in ms. (а) visibility pass, (б) packet build \
         for the observers (on the connection task in production, measured \
         here so the total is complete), (в) `try_send` of the entity \
         messages, (г) the rest of the tick (moves, StreamChunks and their \
         delivery, transfers). 3 s warm-up, 15 s window, 3 repeats.\n",
    );
    for (title, which) in [("p50", P50), ("p99", P99), ("max", MAX)] {
        out.push_str(&format!(
            "\n## {title}\n\n\
             | geometry | P | rep | (а) vis | (б) build | (в) deliver | (г) rest | total |\n\
             |---|---|---|---|---|---|---|---|\n"
        ));
        for run in runs {
            let r = &run.result;
            out.push_str(&format!(
                "| {} | {} | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} |\n",
                run.geometry.label(),
                run.count,
                run.repeat,
                ms(&r.visibility_us, which),
                ms(&r.build_us, which),
                ms(&r.deliver_us, which),
                ms(&r.rest_us, which),
                ms(&r.total_us, which),
            ));
        }
    }

    out.push_str("\n## dominance (p50, mean of the repeats)\n\n");
    for (geometry, count) in [(Geometry::Uniform, 200usize), (Geometry::Dense, 512)] {
        let matching: Vec<&PhaseRun> = runs
            .iter()
            .filter(|run| run.geometry == geometry && run.count == count)
            .collect();
        if matching.is_empty() {
            continue;
        }
        let mean = |pick: fn(&PhaseResult) -> [f64; 4]| -> f64 {
            matching.iter().map(|r| pick(&r.result)[P50]).sum::<f64>() / matching.len() as f64
        };
        let vis = mean(|r| r.visibility_us);
        let build = mean(|r| r.build_us);
        let deliver = mean(|r| r.deliver_us);
        let rest = mean(|r| r.rest_us);
        let total = mean(|r| r.total_us);
        let phases = [
            ("(а) visibility", vis),
            ("(б) build", build),
            ("(в) deliver", deliver),
            ("(г) rest", rest),
        ];
        let (dominant, dominant_us) = phases
            .iter()
            .copied()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .expect("four phases are never empty");
        out.push_str(&format!(
            "- {} P={count}: total p50 {:.3} ms; {} {:.3} ms ({:.0}%), \
             а {:.3} / б {:.3} / в {:.3} / г {:.3} ms.\n",
            geometry.label(),
            total / 1000.0,
            dominant,
            dominant_us / 1000.0,
            dominant_us / total * 100.0,
            vis / 1000.0,
            build / 1000.0,
            deliver / 1000.0,
            rest / 1000.0,
        ));
    }
    out
}

/// Runs the phase-timing series: uniform `P = 200` (the Шаг 3.5 gate) and
/// dense `P = 64/200/512` (for reference), three repeats each, and writes the
/// report. Returns the markdown.
pub fn run_phases_cli() -> anyhow::Result<String> {
    let warmup = Duration::from_secs(3);
    let window = Duration::from_secs(15);
    let configs = [
        (Geometry::Uniform, 200usize),
        (Geometry::Dense, 64),
        (Geometry::Dense, 200),
        (Geometry::Dense, 512),
    ];
    let repeats = 3;
    let mut runs = Vec::new();
    for (geometry, count) in configs {
        for repeat in 1..=repeats {
            let result = measure_phases(geometry, count, warmup, window)?;
            println!(
                "{} P={count} repeat {repeat}: total p50/p99/max {:.3}/{:.3}/{:.3} ms; \
                 а {:.3}/{:.3}/{:.3}; б {:.3}/{:.3}/{:.3}; в {:.3}/{:.3}/{:.3}; \
                 г {:.3}/{:.3}/{:.3} ms",
                geometry.label(),
                ms(&result.total_us, P50),
                ms(&result.total_us, P99),
                ms(&result.total_us, MAX),
                ms(&result.visibility_us, P50),
                ms(&result.visibility_us, P99),
                ms(&result.visibility_us, MAX),
                ms(&result.build_us, P50),
                ms(&result.build_us, P99),
                ms(&result.build_us, MAX),
                ms(&result.deliver_us, P50),
                ms(&result.deliver_us, P99),
                ms(&result.deliver_us, MAX),
                ms(&result.rest_us, P50),
                ms(&result.rest_us, P99),
                ms(&result.rest_us, MAX),
            );
            runs.push(PhaseRun {
                geometry,
                count,
                repeat,
                result,
            });
        }
    }
    let markdown = render_phases(&runs);
    let path = crate::network::save_report("m45-visibility-phases", &markdown)?;
    println!("report: {}", path.display());
    Ok(markdown)
}
