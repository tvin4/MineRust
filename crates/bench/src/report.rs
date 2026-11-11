//! Report plumbing: percentile math, markdown table rows and report files.

use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::BenchSpec;

/// Percentiles of a sample set in µs: p50, p99, p999, max. Samples must be
/// sorted copies — the input is cloned here so callers pass anything.
pub fn percentiles(samples_us: &[f64]) -> [f64; 4] {
    let mut sorted = samples_us.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite tick durations"));
    let pick = |quantile: f64| -> f64 {
        if sorted.is_empty() {
            return 0.0;
        }
        let index = ((sorted.len() as f64) * quantile).floor() as usize;
        sorted[index.min(sorted.len() - 1)]
    };
    [
        pick(0.50),
        pick(0.99),
        pick(0.999),
        *sorted.last().unwrap_or(&0.0),
    ]
}

/// One markdown table row: label, K, region side, players, workload, then
/// the four percentiles and ticks/s.
pub fn run_table(label: &str, spec: BenchSpec, tick_us: [f64; 4], tps: f64) -> String {
    format!(
        "| {label} | {} | {} | {} | {} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} |",
        spec.workers,
        spec.region_side,
        spec.players,
        spec.workload.label(),
        tick_us[0],
        tick_us[1],
        tick_us[2],
        tick_us[3],
        tps,
    )
}

/// Writes one markdown report to `target/bench/<kind>-<label>.md` (not
/// committed) and returns its path.
pub fn write_report(kind: &str, label: &str, markdown: &str) -> std::io::Result<PathBuf> {
    let dir = crate::report_dir();
    std::fs::create_dir_all(&dir)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("{kind}-{label}-{stamp}.md"));
    let mut file = std::fs::File::create(&path)?;
    file.write_all(markdown.as_bytes())?;
    Ok(path)
}
