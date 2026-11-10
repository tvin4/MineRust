//! ЧАСТЬ 0 (M4.4 A): the vanilla join window at four view distances.
//!
//! One run per radius on a fresh copy of the frozen v2 world: the copy's
//! `server.properties` sets `view-distance` and `simulation-distance` to R,
//! the bot's `client_information` asks for the same R, and the bot stands
//! still until no new chunk arrived for 5 s (or 60 s passed). Every chunk
//! coordinate lands in a JSONL trace; the aggregates are read back with
//! `python tools/trace-analyze/analyze.py shape-join <trace>`.
//!
//! ```text
//! cargo test -p parity --test join_window -- --ignored --nocapture
//! ```
//!
//! The test measures; it does not judge. The control R=10 is expected to
//! reproduce the known 473/h=11/K=162 window (`docs/ARCH-M4.md` §4); a radius
//! whose window does not fit the traced circle is a fact to record, never to
//! pad.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parity::{load_parity_env, run_walk_view_at, ParityEnv, Stop, Vanilla, WalkMode};

/// Where the traces land.
const TRACE_DIR: &str = r"C:\MineRust-data\traces";
/// The radii ЧАСТЬ 0 measures (the control is R=10).
const RADII: [i8; 4] = [6, 8, 10, 12];
/// The owner's idle rule and hard budget.
const IDLE: Duration = Duration::from_secs(5);
const TIMEOUT: Duration = Duration::from_secs(60);

/// Runs one radius: fresh v2 copy, both distances set, stand-until-idle.
#[tokio::test]
#[ignore = "requires the owner's installation (C:\\MineRust-data)"]
async fn join_window_at_four_radii() {
    let env = match load_parity_env() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            return;
        }
    };
    if let Err(error) = std::fs::create_dir_all(TRACE_DIR) {
        eprintln!("SKIP: cannot create {TRACE_DIR}: {error}");
        return;
    }
    let ids = Arc::new(env.packet_ids().expect("packet ids"));
    let port = env
        .vanilla_port()
        .expect("server-port in the vanilla server.properties");
    let properties_path = env.vanilla_dir.join("server.properties");
    let original = match std::fs::read_to_string(&properties_path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("SKIP: cannot read {}: {error}", properties_path.display());
            return;
        }
    };

    for radius in RADII {
        let trace = Path::new(TRACE_DIR).join(format!("join-window-r{radius}.jsonl"));
        measure_one(&env, Arc::clone(&ids), port, radius, &trace).await;
    }

    // Restore the shared properties: the file is the vanilla series' own
    // configuration, not this test's to leave changed.
    if let Err(error) = std::fs::write(&properties_path, &original) {
        eprintln!("cannot restore {}: {error}", properties_path.display());
    }
}

/// One radius end to end: set the properties, start vanilla on a fresh v2
/// copy, join, stop, report one aggregate line.
async fn measure_one(
    env: &ParityEnv,
    ids: Arc<data::PacketIds>,
    port: u16,
    radius: i8,
    trace: &Path,
) {
    if let Err(error) = set_view_distance(env, radius) {
        eprintln!("r={radius}: cannot set server.properties: {error}");
        return;
    }
    let server = match Vanilla::start_from(env, &env.world_template).await {
        Ok(server) => server,
        Err(error) => {
            eprintln!("r={radius}: vanilla did not start: {error}");
            return;
        }
    };
    let _ = std::fs::remove_file(trace);
    let mode = WalkMode::JoinWindow {
        idle: IDLE,
        timeout: TIMEOUT,
    };
    let outcome = run_walk_view_at(env, ids, port, radius, mode, trace).await;
    if let Err(error) = server.stop().await {
        eprintln!("r={radius}: vanilla did not stop cleanly: {error}");
    }
    match outcome {
        Ok(stats) => {
            let stop = match stats.stop {
                Stop::WindowIdle { chunks } => format!("WindowIdle chunks={chunks}"),
                Stop::PreloadTimeout { chunks } => format!("TIMEOUT chunks={chunks}"),
                other => format!("{other:?}"),
            };
            eprintln!(
                "r={radius}: {stop}  chunks_seen={}  elapsed={:.0} ms  trace={}",
                stats.chunks,
                stats.elapsed_ms,
                trace.display()
            );
        }
        Err(error) => eprintln!("r={radius}: walk failed: {error}"),
    }
}

/// Rewrites `view-distance` and `simulation-distance` in the copy's
/// `server.properties`, leaving every other key byte for byte.
fn set_view_distance(env: &ParityEnv, radius: i8) -> Result<(), std::io::Error> {
    let path: PathBuf = env.vanilla_dir.join("server.properties");
    let text = std::fs::read_to_string(&path)?;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let replaced = if line.starts_with("view-distance=") {
            format!("view-distance={radius}")
        } else if line.starts_with("simulation-distance=") {
            format!("simulation-distance={radius}")
        } else {
            line.to_owned()
        };
        out.push_str(&replaced);
        out.push('\n');
    }
    std::fs::write(&path, out)
}
