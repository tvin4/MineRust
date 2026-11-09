//! ЧАСТЬ III (M4.0 (а), раунд 2): how long the walk bot waits for terrain —
//! vanilla against our own server, on the same clock (`docs/ROUND2-CONTEXT.md`,
//! п. 4).
//!
//! The bot joins, then in `WalkMode::Preload` it waits for 441
//! `level_chunk_with_light` (the chunks a `view-distance=10` server is
//! expected to send) or 30 seconds, whichever comes first, and leaves. It
//! sends no movement at all, answers every `chunk_batch_finished` with the
//! walk bot's own pace float (`40957d29`, the same in every series — nothing
//! is tuned per server) and prints nothing the traces do not also contain.
//!
//! Four series of `REPEATS` runs:
//!
//! * **А) vanilla cold** — `Vanilla::start` for every run: a fresh copy of
//!   the frozen v2 template *and* the first join of a fresh process;
//! * **Б) vanilla warm** — one live server, one unmeasured join first (the
//!   owner's «второй вход на живом сервере»), then the three measured ones;
//! * **В) our server, cold** — `cargo run -p server --release` on 25565
//!   started per run; the world is read-only, our server never writes a save;
//! * **В) our server, warm** — the same binary started once, one unmeasured
//!   join, then the three measured ones.
//!
//! Facts only: no run asserts that one server beats another, and the report
//! file says so itself. The numbers come from each run's own trace, so
//! `traces\preload-report.md` can be recomputed from the JSONL beside it.
//!
//! ```text
//! cargo run -p server --release   (the test runs it itself, cwd = workspace)
//! cargo test -p parity --test preload_measure -- --ignored --nocapture
//! ```

#[path = "common/preload_report.rs"]
mod preload_report;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use parity::{
    load_parity_env, run_walk_at, ParityEnv, PreloadMetrics, Vanilla, WalkMode, PRELOAD_TIMEOUT,
    VIEW_DISTANCE_CHUNKS,
};
use preload_report::{render, Measured, Scene, REPEATS, TRACE_DIR};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};

/// Our own port (`server.toml`: `listen = "0.0.0.0:25565"`).
const OUR_PORT: u16 = 25565;
/// What one preload run waits for and for how long.
const TARGET: u64 = VIEW_DISTANCE_CHUNKS;
const BUDGET: Duration = PRELOAD_TIMEOUT;
/// Release-build budget: `cargo run --server --release` compiles first.
const OUR_START_TIMEOUT: Duration = Duration::from_secs(600);
/// The report file next to the traces.
const REPORT_NAME: &str = "preload-report.md";

/// The four series, then one report file with the conditions and every
/// run spelled out.
#[tokio::test]
#[ignore = "requires the owner's installation (C:\\MineRust-data)"]
async fn entry_speed_measurements() {
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

    let mut scenes = Vec::new();
    scenes.push(vanilla_cold(&env, Arc::clone(&ids), vanilla_port).await);
    scenes.push(vanilla_warm(&env, Arc::clone(&ids), vanilla_port).await);
    scenes.push(ours_cold(&env, Arc::clone(&ids), "preload-ours").await);
    scenes.push(ours_warm(&env, Arc::clone(&ids), "preload-ours").await);

    let header = conditions(&env, vanilla_port);
    let path = Path::new(TRACE_DIR).join(REPORT_NAME);
    match std::fs::write(&path, render(&header, &scenes)) {
        Ok(()) => eprintln!("report: {}", path.display()),
        Err(error) => eprintln!("cannot write {}: {error}", path.display()),
    }
}

/// ЧАСТЬ II, п. 2: only our own series are re-measured, now that the
/// scripted known-packs wait is conditional. The vanilla series are
/// deliberately NOT re-run: they are the previous measurement the fix is
/// compared against, and the code path they take did not change at all.
#[tokio::test]
#[ignore = "requires the owner's installation (C:\\MineRust-data)"]
async fn our_series_after_probe_wait_fix() {
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

    let scenes = vec![
        ours_cold(&env, Arc::clone(&ids), "preload2-ours").await,
        ours_warm(&env, Arc::clone(&ids), "preload2-ours").await,
    ];

    let header = format!(
        "{}\n\
         - ЧАСТЬ II, п. 2: the scripted known-packs wait is conditional now (the full budget \n\
         only for the vanilla port, 100 ms otherwise), so these two series carry no bot-side \n\
         3 s wait; the vanilla series are NOT re-run and stay as measured before the fix.\n",
        conditions(&env, vanilla_port)
    );
    let path = Path::new(TRACE_DIR).join("preload-report-ours-2.md");
    match std::fs::write(&path, render(&header, &scenes)) {
        Ok(()) => eprintln!("report: {}", path.display()),
        Err(error) => eprintln!("cannot write {}: {error}", path.display()),
    }
}

/// One measured join at `port`: the bot waits for the target, then the
/// trace is read back into the run's facts.
async fn measurement(env: &ParityEnv, ids: Arc<PacketIds>, port: u16, name: &str) -> Measured {
    let trace = Path::new(TRACE_DIR).join(format!("{name}.jsonl"));
    let mut run = Measured::new(name);
    run.trace = trace.clone();
    // A stale trace would otherwise be measured as this run's own.
    let _ = std::fs::remove_file(&trace);

    let mode = WalkMode::Preload {
        chunks: TARGET,
        timeout: BUDGET,
    };
    run.stats = match run_walk_at(env, ids, port, mode, &trace).await {
        Ok(stats) => Some(stats),
        Err(error) => {
            run.fatal = Some(error.to_string());
            return run;
        }
    };
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

/// А) vanilla, fresh copy of the v2 template and a fresh process per run.
async fn vanilla_cold(env: &ParityEnv, ids: Arc<PacketIds>, port: u16) -> Scene {
    let mut scene = Scene::new("А) vanilla cold — fresh v2 copy, fresh process, first join");
    for index in 1..=REPEATS {
        let name = format!("preload-vanilla-cold-{index}");
        let server = match Vanilla::start(env).await {
            Ok(server) => server,
            Err(error) => {
                scene.runs.push(failed(&name, error));
                continue;
            }
        };
        let run = measurement(env, Arc::clone(&ids), port, &name).await;
        // The measurement stands on its own even if the server did not go
        // down cleanly; a leftover process would then fail *later* runs,
        // which is recorded as itself.
        if let Err(error) = server.stop().await {
            eprintln!("{name}: vanilla stop failed: {error}");
        }
        scene.runs.push(run);
    }
    scene
}

/// Б) vanilla, one live server: an unmeasured join first (the owner's
/// «второй вход»), then the three measured ones.
async fn vanilla_warm(env: &ParityEnv, ids: Arc<PacketIds>, port: u16) -> Scene {
    let mut scene = Scene::new("Б) vanilla warm — one live server, joins 2..4");
    let server = match Vanilla::start(env).await {
        Ok(server) => server,
        Err(error) => {
            scene.runs.push(failed("preload-vanilla-warm-1", error));
            return scene;
        }
    };
    eprintln!("vanilla warm-up join (not measured)…");
    let warmup = measurement(env, Arc::clone(&ids), port, "preload-vanilla-warm-0").await;
    if let Some(error) = warmup.fatal.as_ref() {
        eprintln!("warm-up join failed: {error}");
    }
    for index in 1..=REPEATS {
        let name = format!("preload-vanilla-warm-{index}");
        scene
            .runs
            .push(measurement(env, Arc::clone(&ids), port, &name).await);
    }
    if let Err(error) = server.stop().await {
        eprintln!("vanilla stop failed: {error}");
    }
    scene
}

/// В) our server, cold: one release process per run, first join of that
/// process (the server reads the template, it never writes a save).
async fn ours_cold(env: &ParityEnv, ids: Arc<PacketIds>, prefix: &str) -> Scene {
    let mut scene = Scene::new("В) our server cold — fresh release process, first join");
    for index in 1..=REPEATS {
        let name = format!("{prefix}-cold-{index}");
        scene.runs.push(then_measure(&name, env, ids.clone()).await);
    }
    scene
}

/// В) our server, warm: one release process, an unmeasured join first,
/// then the three measured ones on the live process.
async fn ours_warm(env: &ParityEnv, ids: Arc<PacketIds>, prefix: &str) -> Scene {
    let first = format!("{prefix}-warm-1");
    let mut scene = Scene::new("В) our server warm — one live release process, joins 2..4");
    let server = match start_ours().await {
        Ok(server) => server,
        Err(error) => {
            scene.runs.push(failed(&first, error));
            return scene;
        }
    };
    if !wait_for_port(OUR_PORT, OUR_START_TIMEOUT).await {
        scene
            .runs
            .push(failed(&first, "our release server did not listen in time"));
        stop_ours(server).await;
        return scene;
    }
    eprintln!("our warm-up join (not measured)…");
    let warmup = format!("{prefix}-warm-0");
    measurement(env, Arc::clone(&ids), OUR_PORT, &warmup).await;
    for index in 1..=REPEATS {
        let name = format!("{prefix}-warm-{index}");
        scene
            .runs
            .push(measurement(env, Arc::clone(&ids), OUR_PORT, &name).await);
    }
    stop_ours(server).await;
    scene
}

/// One start → join → stop round of the cold series: `None` when our own
/// server could not be started at all.
async fn then_measure(name: &str, env: &ParityEnv, ids: Arc<PacketIds>) -> Measured {
    let server = match start_ours().await {
        Ok(server) => server,
        Err(error) => return failed(name, error),
    };
    if !wait_for_port(OUR_PORT, OUR_START_TIMEOUT).await {
        stop_ours(server).await;
        return failed(name, "our release server did not listen in time");
    }
    let run = measurement(env, ids, OUR_PORT, name).await;
    stop_ours(server).await;
    run
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
         441-chunk target, {BUDGET:?} budget\n\
         - clock: a trace's `t_ms` is milliseconds since the connection attempt (walk \
         `ms()`), fixed in b74ac44 — older traces cannot be compared with these\n",
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
