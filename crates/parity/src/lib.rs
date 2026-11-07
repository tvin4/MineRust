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
mod env;
mod trace;
mod vanilla;

pub use client::{Client, ClientError, Step};
pub use env::{load_parity_env, EnvError, ParityEnv, PACKETS_REPORT_VAR, SERVER_TOML_VAR};
pub use trace::{
    from_hex, to_hex, HexError, Trace, TraceDirection, TraceEntry, TraceIoError, TraceState,
};
pub use vanilla::{copy_dir_all, Vanilla, VanillaError};
