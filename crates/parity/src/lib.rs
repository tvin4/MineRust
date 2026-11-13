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
mod proxy;
mod steps;
mod trace;
mod vanilla;
/// Public because the bench bot reuses its packet helpers (`teleport_ack`,
/// the move payload layouts) without re-deriving them (M4.6, ARCH-M4 §5).
pub mod walk;

pub use client::{Client, ClientError, Step};
pub use diff::{diff_traces, normalize, render_report, CountRow, PayloadRow, TracesDiff};
pub use env::{load_parity_env, EnvError, ParityEnv, PACKETS_REPORT_VAR, SERVER_TOML_VAR};
pub use preload::{
    min_median_max, PreloadError, PreloadMetrics, PRELOAD_TIMEOUT, VIEW_DISTANCE_CHUNKS,
};
pub use proxy::{run_proxy, ProxyConfig, ProxyError};
pub use steps::{
    join_steps, join_steps_named, join_steps_with_view, PROBE_WAIT, PROBE_WAIT_NO_PROBE,
};
pub use trace::{
    from_base64, from_hex, to_hex, HexError, Trace, TraceDirection, TraceEntry, TraceIoError,
    TraceState,
};
pub use vanilla::{copy_dir_all, Vanilla, VanillaError};
pub use walk::{
    run_walk, run_walk_at, run_walk_metrics_at, run_walk_view_at, Batch, CacheMove, RunStats, Stop,
    WalkError, WalkMode, PACE, UNKNOWN_CHUNK, VIEW_DISTANCE,
};
