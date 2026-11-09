//! ЧАСТЬ II (M4.0 (а), раунд 4): is the bot itself the limit?
//!
//! The warm entry-time series is repeated with the same bot, the same
//! 441-chunk target, the same pace float (`40957d29`, ≈ 4.6715) and the
//! same server processes, but in two recording modes:
//!
//! * **metrics-only** — no JSONL and no base64 during the run: counters,
//!   payload sizes and monotonic receive stamps stay in memory, and one
//!   small JSON file is written after the bot left;
//! * **JSONL** — the traced mode every earlier series used (one base64
//!   line per packet).
//!
//! The bot also sums how long it was parked inside `recv_packet`, so the
//! report can show the share of the run it did **not** read the socket.
//! If the metrics-only time is notably lower, the old numbers were limited
//! by the bot — the report says so instead of silently replacing them.
//!
//! Vanilla and our own release server are measured on one live process
//! each (warm: one unmeasured join first). The two modes alternate within
//! a server so cache warm-up cannot favour the mode that runs later.
//!
//! ```text
//! cargo test -p parity --test metrics_measure -- --ignored --nocapture
//! ```

#[path = "common/preload_report.rs"]
mod preload_report;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use parity::{
    load_parity_env, run_walk_at, run_walk_metrics_at, ParityEnv, PreloadMetrics, Vanilla,
    WalkMode, PACE, PRELOAD_TIMEOUT, VIEW_DISTANCE_CHUNKS,
};
use preload_report::{render, Measured, Scene, REPEATS, TRACE_DIR};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};

/// Our own port (`server.toml`: `listen = "0.0.0.0:25565"`).
const OUR_PORT: u16 = 25565;
/// What one preload run waits for and for how long.
const TARGET: u64 = VIEW_DISTANCE_CHUNKS;
const BUDGET: Duration = PRELOAD_TIMEOUT;
/// Release-build budget: `cargo run -p server --release` compiles first.
const OUR_START_TIMEOUT: Duration = Duration::from_secs(600);
/// The report file next to the traces.
const REPORT_NAME: &str = "part2-report.md";

/// The four series: {vanilla, ours} × {metrics-only, JSONL}, three runs
/// each, on one warm server process per side.
#[tokio::test]
#[ignore = "requires the owner's installation (C:\\MineRust-data)"]
async fn metrics_only_series() {
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
    let vanilla_port = env
        .vanilla_port()
        .expect("server-port in the vanilla server.properties");

    let mut scenes = vanilla_series(&env, Arc::clone(&ids), vanilla_port).await;
    scenes.extend(ours_series(&env, Arc::clone(&ids)).await);

    let header = format!(
        "{}\n\
         - ЧАСТЬ II: each side gets three metrics-only joins and three JSONL joins \
         (the classic traced bot) in alternating order on the same live process, all \
         with pace `40957d29` and the 441-chunk target. The metrics-only bot writes \
         no JSONL and no base64 during the run; its file is written after it leaves.\n",
        conditions(&env, vanilla_port)
    );
    let path = Path::new(TRACE_DIR).join(REPORT_NAME);
    match std::fs::write(&path, render(&header, &scenes)) {
        Ok(()) => eprintln!("report: {}", path.display()),
        Err(error) => eprintln!("cannot write {}: {error}", path.display()),
    }
}

/// The preload wait both modes share.
fn preload_mode() -> WalkMode {
    WalkMode::Preload {
        chunks: TARGET,
        timeout: BUDGET,
        pace: PACE,
    }
}

/// One metrics-only join: no trace, one JSON file after the run.
async fn measure_metrics(env: &ParityEnv, ids: Arc<PacketIds>, port: u16, name: &str) -> Measured {
    let path = Path::new(TRACE_DIR).join(format!("{name}.metrics.json"));
    // A stale file must never be read as this run's own result.
    let _ = std::fs::remove_file(&path);
    let mut run = Measured::new(name);
    run.trace = path.clone();
    match run_walk_metrics_at(env, ids, port, preload_mode(), &path).await {
        Ok((stats, metrics)) => {
            run.stats = Some(stats);
            run.metrics = Some(metrics);
        }
        Err(error) => run.fatal = Some(error.to_string()),
    }
    eprintln!("{}: {}", run.name, run.stop_and_chunks());
    run
}

/// One classic JSONL join, read back into the same facts.
async fn measure_jsonl(env: &ParityEnv, ids: Arc<PacketIds>, port: u16, name: &str) -> Measured {
    let trace = Path::new(TRACE_DIR).join(format!("{name}.jsonl"));
    let _ = std::fs::remove_file(&trace);
    let mut run = Measured::new(name);
    run.trace = trace.clone();
    match run_walk_at(env, ids, port, preload_mode(), &trace).await {
        Ok(stats) => run.stats = Some(stats),
        Err(error) => {
            run.fatal = Some(error.to_string());
            return run;
        }
    }
    match PreloadMetrics::from_trace(&trace, TARGET) {
        Ok(metrics) => run.metrics = Some(metrics),
        Err(error) => run.fatal = Some(format!("trace read: {error}")),
    }
    eprintln!("{}: {}", run.name, run.stop_and_chunks());
    run
}

/// A run that never happened (its server did not start).
fn failed(name: &str, error: impl std::fmt::Display) -> Measured {
    let mut run = Measured::new(name);
    run.fatal = Some(error.to_string());
    eprintln!("{name}: no run ({error})");
    run
}

/// Vanilla warm: one live server, an unmeasured join, then metrics and
/// JSONL alternating.
async fn vanilla_series(env: &ParityEnv, ids: Arc<PacketIds>, port: u16) -> Vec<Scene> {
    let mut metrics = Scene::new("II) vanilla warm, metrics-only");
    let mut jsonl = Scene::new("II) vanilla warm, JSONL (the classic bot)");
    let server = match Vanilla::start(env).await {
        Ok(server) => server,
        Err(error) => {
            metrics.runs.push(failed("part2-vanilla-metrics-1", &error));
            return vec![metrics, jsonl];
        }
    };
    eprintln!("vanilla warm-up join (not measured)...");
    let warmup = measure_jsonl(env, Arc::clone(&ids), port, "part2-vanilla-warmup").await;
    if let Some(error) = warmup.fatal.as_ref() {
        eprintln!("vanilla warm-up join failed: {error}");
    }
    for index in 1..=REPEATS {
        metrics.runs.push(
            measure_metrics(
                env,
                Arc::clone(&ids),
                port,
                &format!("part2-vanilla-metrics-{index}"),
            )
            .await,
        );
        jsonl.runs.push(
            measure_jsonl(
                env,
                Arc::clone(&ids),
                port,
                &format!("part2-vanilla-jsonl-{index}"),
            )
            .await,
        );
    }
    if let Err(error) = server.stop().await {
        eprintln!("vanilla stop failed: {error}");
    }
    vec![metrics, jsonl]
}

/// Our warm: one live release process, an unmeasured join, then metrics
/// and JSONL alternating.
async fn ours_series(env: &ParityEnv, ids: Arc<PacketIds>) -> Vec<Scene> {
    let mut metrics = Scene::new("II) our warm, metrics-only");
    let mut jsonl = Scene::new("II) our warm, JSONL (the classic bot)");
    let server = match start_ours().await {
        Ok(server) => server,
        Err(error) => {
            metrics.runs.push(failed("part2-ours-metrics-1", &error));
            return vec![metrics, jsonl];
        }
    };
    if !wait_for_port(OUR_PORT, OUR_START_TIMEOUT).await {
        metrics.runs.push(failed(
            "part2-ours-metrics-1",
            "our release server did not listen in time",
        ));
        stop_ours(server).await;
        return vec![metrics, jsonl];
    }
    eprintln!("our warm-up join (not measured)...");
    let warmup = measure_jsonl(env, Arc::clone(&ids), OUR_PORT, "part2-ours-warmup").await;
    if let Some(error) = warmup.fatal.as_ref() {
        eprintln!("our warm-up join failed: {error}");
    }
    for index in 1..=REPEATS {
        metrics.runs.push(
            measure_metrics(
                env,
                Arc::clone(&ids),
                OUR_PORT,
                &format!("part2-ours-metrics-{index}"),
            )
            .await,
        );
        jsonl.runs.push(
            measure_jsonl(
                env,
                Arc::clone(&ids),
                OUR_PORT,
                &format!("part2-ours-jsonl-{index}"),
            )
            .await,
        );
    }
    stop_ours(server).await;
    vec![metrics, jsonl]
}

/// Builds (once — `cargo run` reuses the artifact) and starts our own
/// binary: `cargo run -p server --release`, workspace root as its working
/// directory, because `server.toml` is looked up from there.
async fn start_ours() -> std::io::Result<Child> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    Command::new("cargo")
        .args(["run", "-p", "server", "--release"])
        .current_dir(&root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
}

/// Waits until something accepts the connection on `port`.
async fn wait_for_port(port: u16, timeout: Duration) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let until = tokio::time::Instant::now() + timeout;
    loop {
        let attempt = tokio::time::timeout(Duration::from_millis(500), TcpStream::connect(addr));
        if let Ok(Ok(_stream)) = attempt.await {
            return true;
        }
        if tokio::time::Instant::now() >= until {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Stops our server: `cargo run` is only a wrapper, so the listener (its
/// child) goes down with the whole tree — the binary has no console input.
async fn stop_ours(mut server: Child) {
    let Some(pid) = server.id() else {
        let _ = server.kill().await;
        return;
    };
    let kill = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .status()
        .await;
    match kill {
        Ok(status) => {
            let _ = server.wait().await;
            if !status.success() {
                eprintln!("taskkill did not stop {pid}: {status}");
            }
        }
        Err(error) => {
            let _ = server.kill().await;
            eprintln!("taskkill failed for {pid}: {error}");
        }
    }
}

/// Conditions of every series, read out of the files instead of typed from
/// memory: ports, compression, online mode, view distance, pace.
fn conditions(env: &ParityEnv, vanilla_port: u16) -> String {
    let properties = std::fs::read_to_string(env.vanilla_dir.join("server.properties"))
        .unwrap_or_else(|error| format!("(server.properties unreadable: {error})"));
    let property = |key: &str| -> String {
        properties
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .unwrap_or("(not set)")
            .trim()
            .to_owned()
    };
    format!(
        "- vanilla: port {vanilla_port} (`server-port={}`), `network-compression-threshold={}` \
         (a negative value disables compression), `online-mode={}`, `gamemode={}` with \
         `force-gamemode={}`, `view-distance={}`\n\
         - ours: `cargo run -p server --release` on port {OUR_PORT} (`{}` in server.toml), \
         view distance {}, compression: crates/net never sends `SetCompression`, so no \
         threshold exists at all\n\
         - both: ← same bot, same pace float `40957d29` (≈4.6715 chunks per tick), same \
         441-chunk target, {BUDGET:?} budget, buffered socket in both recording modes\n\
         - clock: a trace's `t_ms` is milliseconds since the connection attempt; the \
         metrics-only file uses the same clock\n",
        property("server-port="),
        property("network-compression-threshold="),
        property("online-mode="),
        property("gamemode="),
        property("force-gamemode="),
        property("view-distance="),
        env.config.listen,
        env.config.view_distance,
    )
}
