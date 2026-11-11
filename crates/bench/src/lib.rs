//! The M4.6 benchmark library (ARCH-M4 §5): engines, players and reports.
//!
//! The binary in `main.rs` is the CLI shell; the smoke test imports this
//! lib directly. See the crate root doc in `main.rs` for the modes.

pub mod bot;
pub mod capacity;
pub mod network;
pub mod paced;
pub mod players;
pub mod report;
pub mod world;

use std::path::PathBuf;
use std::time::Duration;

/// One run's CLI shape: the subset of the matrix the mode needs.
#[derive(Debug, Clone, Copy)]
pub struct BenchSpec {
    /// Worker threads of the region pool (`0` = inline, no pool).
    pub workers: usize,
    /// Region grid side in chunks (`0` = no split, one region).
    pub region_side: i32,
    /// Players distributed over the grid.
    pub players: usize,
    /// The workload of every region this tick.
    pub workload: Workload,
    /// Warm-up ticks discarded (both engines).
    pub warmup: u64,
    /// Measured ticks (capacity).
    pub ticks: u64,
    /// Measured seconds (paced).
    pub seconds: u64,
}

/// The per-region load added on top of W1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workload {
    /// The real tick only: move decode, StreamChunks, Transfer.
    W1,
    /// W1 plus a busy-loop of `S` µs per region per tick. **Not** game
    /// speed: the scalability ceiling of the mechanism.
    W2(u64),
}

impl Workload {
    /// Duration of the busy-loop part, zero for W1.
    pub fn duration(self) -> Duration {
        match self {
            Workload::W1 => Duration::ZERO,
            Workload::W2(micros) => Duration::from_micros(micros),
        }
    }

    /// Stable label for tables and file names.
    pub fn label(self) -> &'static str {
        match self {
            Workload::W1 => "W1",
            Workload::W2(50) => "W2s50",
            Workload::W2(200) => "W2s200",
            Workload::W2(1000) => "W2s1000",
            Workload::W2(_) => "W2",
        }
    }
}

impl Default for BenchSpec {
    fn default() -> Self {
        Self {
            workers: default_workers(),
            region_side: 64,
            players: 64,
            workload: Workload::W1,
            warmup: 100,
            ticks: 200,
            seconds: 10,
        }
    }
}

/// `available_parallelism − 2`, at least 1 — the D5 default the config uses.
pub fn default_workers() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get().saturating_sub(2).max(1))
}

/// Where the markdown reports go.
pub fn report_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target"))
        .join("bench")
}
