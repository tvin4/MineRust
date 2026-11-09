//! ЧАСТЬ III: what one preload series produced — facts only.
//!
//! The report builder of `tests/preload_measure.rs`. Everything here is a
//! number taken from a trace or from the bot's own statistics; the table
//! makes no claim about which server is faster, which the owner explicitly
//! wants as a separate step (`docs/ROUND2-CONTEXT.md`, п. 4).

use std::path::PathBuf;

use parity::{min_median_max, PreloadMetrics, RunStats};

/// Where the traces and reports of the measurements land (outside the
/// repository, by design — the same rule as the walk runs).
pub const TRACE_DIR: &str = r"C:\MineRust-data\traces";

/// Repeats per series (the owner's «×3»), shared with the driver.
pub const REPEATS: usize = 3;

/// One completed (or failed) preload run.
pub struct Measured {
    /// Trace/report file stem, e.g. `preload-vanilla-cold-1`.
    pub name: String,
    /// The JSONL the bot wrote (empty when no run happened at all).
    pub trace: PathBuf,
    /// The bot's own statistics; `None` when the run failed before play.
    pub stats: Option<RunStats>,
    /// The entry-speed facts read back from that trace.
    pub metrics: Option<PreloadMetrics>,
    /// Why there is no run (connect/join/trace error).
    pub fatal: Option<String>,
}

impl Measured {
    /// A run with no results yet: the name is known, the trace path is set
    /// by the caller right after.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            trace: PathBuf::new(),
            stats: None,
            metrics: None,
            fatal: None,
        }
    }

    /// The two facts a table row may always print: stop reason and how
    /// many chunks the bot counted itself (independent of the trace).
    pub fn stop_and_chunks(&self) -> String {
        let Some(fatal) = &self.fatal else {
            return match &self.stats {
                Some(stats) => {
                    let stop = format!("{:?}", stats.stop);
                    let chunks = stats.chunks;
                    format!(
                        "{stop}, {chunks} chunks, {:.0} ms of play",
                        stats.elapsed_ms
                    )
                }
                None => "no statistics".to_owned(),
            };
        };
        format!("no run: {fatal}")
    }
}

/// One measurement series (the owner's А), Б) or В)), the repeats in the
/// order they were made.
pub struct Scene {
    /// Human label, the owner's own wording.
    pub label: &'static str,
    /// The repeats.
    pub runs: Vec<Measured>,
}

impl Scene {
    /// A new empty series.
    pub fn new(label: &'static str) -> Self {
        Self {
            label,
            runs: Vec::new(),
        }
    }
}

/// Median of one vector of gaps; `None` without at least two of them.
fn median(values: &[f64]) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("measured gaps are finite"));
    let len = sorted.len();
    match len {
        0 => None,
        1 => None,
        _ if len % 2 == 1 => Some(sorted[len / 2]),
        _ => Some((sorted[len / 2 - 1] + sorted[len / 2]) / 2.0),
    }
}

/// Mean chunks in one finished batch of that run.
fn mean_batch(metrics: &PreloadMetrics) -> Option<f64> {
    if metrics.batch_finishes == 0 {
        return None;
    }
    let chunks_per_batch = metrics.chunks as f64;
    Some(chunks_per_batch / f64::from(metrics.batch_finishes as u32))
}

/// Median gap between that run's own `chunk_batch_finished` packets.
fn in_run_median_gap(metrics: &PreloadMetrics) -> Option<f64> {
    median(&metrics.batch_gaps_ms)
}

/// Every metric of the owner's table, in order: label, one value per run
/// (`None` when that run did not produce it), printed precision, unit.
fn metrics(scene: &Scene) -> Vec<(String, Vec<Option<f64>>, usize, &'static str)> {
    let runs = &scene.runs;
    let pick = |f: fn(&PreloadMetrics) -> Option<f64>| -> Vec<Option<f64>> {
        runs.iter()
            .map(|run| run.metrics.as_ref().and_then(f))
            .collect()
    };
    vec![
        (
            "intention → login (play)".to_owned(),
            pick(PreloadMetrics::intention_to_login_ms),
            1,
            "ms",
        ),
        (
            "intention → first level_chunk_with_light".to_owned(),
            pick(PreloadMetrics::intention_to_first_chunk_ms),
            1,
            "ms",
        ),
        (
            "intention → last chunk_batch_finished".to_owned(),
            pick(PreloadMetrics::intention_to_last_batch_ms),
            1,
            "ms",
        ),
        (
            "intention → 441st level_chunk_with_light".to_owned(),
            pick(PreloadMetrics::intention_to_target_ms),
            1,
            "ms",
        ),
        (
            "level_chunk_with_light".to_owned(),
            pick(|m| Some(m.chunks as f64)),
            0,
            "pkt",
        ),
        (
            "chunk_batch_finished".to_owned(),
            pick(|m| Some(m.batch_finishes as f64)),
            0,
            "pkt",
        ),
        (
            "chunk payload bytes".to_owned(),
            pick(|m| Some(m.chunk_bytes as f64)),
            0,
            "B",
        ),
        (
            "chunks per batch (mean of the run)".to_owned(),
            pick(mean_batch),
            1,
            "chunks",
        ),
        (
            "batch gap (median inside the run)".to_owned(),
            pick(in_run_median_gap),
            1,
            "ms",
        ),
    ]
}

/// `min | median | max` of one metric; an absent value prints as `—`.
fn spread_row(label: &str, values: &[Option<f64>], digits: usize, unit: &str) -> String {
    let n = values.iter().flatten().count();
    match min_median_max(values) {
        Some((min, median, max)) => format!(
            "| {label} | {n} | {min:.digits$} {unit} | {median:.digits$} {unit} | {max:.digits$} {unit} |\n"
        ),
        None => format!("| {label} | {n} | — | — | — |\n"),
    }
}

/// The table of one series plus its runs spelled out underneath.
pub fn scene_block(scene: &Scene) -> String {
    let mut text = format!("### {}\n\n", scene.label);
    text.push_str(&format!("runs: {} of {REPEATS}\n\n", scene.runs.len()));
    text.push_str("| metric | n | min | median | max |\n|---|---:|---:|---:|---:|\n");
    for (label, values, digits, unit) in metrics(scene) {
        text.push_str(&spread_row(&label, &values, digits, unit));
    }
    text.push_str("\nper run:\n\n");
    for run in &scene.runs {
        text.push_str(&format!("- `{}`: {}\n", run.name, run.stop_and_chunks()));
        let mode = run.stats.as_ref().map(|stats| {
            format!(
                "game_mode {:?}, player_abilities flags {:?} (recorded, never judged)",
                stats.game_mode, stats.abilities_flags
            )
        });
        text.push_str(&format!("  trace: {}\n", run.trace.display()));
        if let Some(mode) = mode {
            text.push_str(&format!("  mode facts: {mode}\n"));
        }
    }
    text.push('\n');
    text
}

/// The whole report: the conditions first (nobody may mistake these runs
/// for something else), then one block per series.
///
/// The closing paragraph is part of the tool: the owner asked for no
/// faster/slower conclusion without handling the differences first
/// (`docs/ROUND2-CONTEXT.md`, п. 4), and this file cannot draw it anyway.
pub fn render(header: &str, scenes: &[Scene]) -> String {
    let mut text = String::from("# M4.0 (а), ЧАСТЬ III — entry speed: facts only\n\n");
    text.push_str(header);
    text.push_str(
        "\nEvery time below is the walk bot's own monotonic clock, in milliseconds \
         since that run's connection attempt; the same numbers can be recomputed \
         from the JSONL trace on the same line of this table.\n\n",
    );
    for scene in scenes {
        text.push_str(&scene_block(scene));
    }
    text.push_str("\n## What this file does not say\n\n");
    text.push_str(
        "Nothing here is faster or slower than anything else: the four series differ in \
         the world state (cold/warm), in whatever the server had to do first, and in \
         which process served them. Those are hypotheses for the final report, not \
         conclusions of this one. Vanilla preloads its spawn area at start, so its \
         \"cold\" is only a first join of a fresh world copy.\n",
    );
    text
}
