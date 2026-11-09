//! Parity testing against the official vanilla server (milestone M3).
//!
//! The idea (see `docs/SOURCES.md`, «План: крейт parity»): the same scripted
//! test client talks to a real vanilla `server.jar` and to our own server,
//! both on a fresh copy of the same frozen world, and the recorded packet
//! traces are diffed. Everything in this crate is manual-only: the tests are
//! `#[ignore]`d because they need the owner's vanilla installation
//! (`C:\MineRust-data`), which is deliberately outside the repository.
//!
//! Usage:
//!
//! ```text
//! cargo test -p parity -- --ignored
//! ```
//!
//! Skipping is graceful: without `vanilla_server_jar`/`vanilla_server_dir`
//! in `server.toml` the tests print the configuration error and return.

mod client;
mod diff;
mod env;
mod preload;
mod steps;
mod trace;
mod vanilla;
mod walk;

pub use client::{Client, ClientError, Step};
pub use diff::{diff_traces, normalize, render_report, CountRow, PayloadRow, TracesDiff};
pub use env::{load_parity_env, EnvError, ParityEnv, PACKETS_REPORT_VAR, SERVER_TOML_VAR};
pub use preload::{
    min_median_max, PreloadError, PreloadMetrics, PRELOAD_TIMEOUT, VIEW_DISTANCE_CHUNKS,
};
pub use steps::{join_steps, join_steps_with_view};
pub use trace::{
    from_hex, to_hex, HexError, Trace, TraceDirection, TraceEntry, TraceIoError, TraceState,
};
pub use vanilla::{copy_dir_all, Vanilla, VanillaError};
pub use walk::{
    run_walk, run_walk_at, Batch, CacheMove, RunStats, Stop, WalkError, WalkMode, UNKNOWN_CHUNK,
    VIEW_DISTANCE,
};
