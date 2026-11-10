//! M4.4 B, ШАГ 4: the walk-streaming check on the frozen v3 world against
//! our own in-process server.
//!
//! Two ignored tests, both needing the owner's installation:
//!
//! * `window_v3_warmup_is_within_budget` — builds the in-process world on
//!   `world-template-v3`, loads its ready batch once and reports the
//!   warm-up time, the ready-chunk count, the total cache bytes and an
//!   approximate working set. Stop conditions (the owner's): warm-up > 20 s
//!   or cache > 1.5 GB.
//! * `window_v3_walk_matches_vanilla` — starts the in-process server with a
//!   caller-owned tick-metrics handle, then runs the П4 (straight +x) and П5
//!   (diagonal +x/+z) walks of 600 × 0.2-block steps with the spectator
//!   check off (our server is survival on purpose). The JSONL traces land in
//!   `C:\MineRust-data\traces\window-p{4,5}-ours.jsonl` so
//!   `python tools/trace-analyze/analyze.py` can diff them against the
//!   vanilla `window-p{4,5}.jsonl`; tick p50/p99/max and `late_ticks` are
//!   read from the metrics handle and printed.
//!
//! ```text
//! cargo test -p parity --release --test window_walk_ours -- --ignored --nocapture
//! ```
//!
//! Run it with `--release`: the world reader is CPU-bound, and the debug
//! build is about six times slower (v3 warm-up ~85 s debug vs ~13.5 s
//! release). The budget is the production one; the debug build only warns.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parity::{load_parity_env, run_walk_view_at, ParityEnv, Stop, WalkMode};

/// Where the traces land, next to the vanilla П4/П5 traces.
const TRACE_DIR: &str = r"C:\MineRust-data\traces";
/// The frozen v3 world (65×65 full chunks), untouched by these runs: the
/// in-process server only reads it.
const V3_TEMPLATE: &str = r"C:\MineRust-data\world-template-v3";
/// Idle after the last step (Part I kept 5 s).
const SETTLE: Duration = Duration::from_secs(5);
/// П4/П5: 600 steps of 0.2 blocks on the 50 ms grid ≈ 120 blocks.
const STEPS: u32 = 600;
/// Owner's stop conditions for the warm-up.
const WARM_BUDGET: Duration = Duration::from_secs(20);
/// 1.5 GB of ready payloads.
const CACHE_BUDGET_BYTES: u64 = 1_500_000_000;

/// Builds the config pointed at v3, or `None` when the template is absent.
fn v3_config(env: &ParityEnv) -> Option<server::ServerConfig> {
    if !Path::new(V3_TEMPLATE).is_dir() {
        eprintln!("SKIP: v3 template not found at {V3_TEMPLATE}");
        return None;
    }
    let mut config = env.config.clone();
    config.world_dir = Some(V3_TEMPLATE.to_owned());
    Some(config)
}

/// Approximate working set of this process in bytes, read through
/// PowerShell. `None` when the probe is unavailable (the report then says
/// so instead of inventing a number).
fn working_set_bytes() -> Option<u64> {
    let pid = std::process::id();
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("(Get-Process -Id {pid}).WorkingSet64"),
        ])
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

/// Loads the game data from the owner's client jar.
fn game_data(env: &ParityEnv) -> Arc<data::GameData> {
    Arc::new(
        data::load_from_jar(
            &env.config.client_jar,
            env.config.registries_report.as_deref().map(Path::new),
        )
        .expect("game data from the jar"),
    )
}

/// ШАГ 4.1: the v3 warm-up measurement, with the owner's stop conditions.
#[tokio::test]
#[ignore = "requires the owner's installation (C:\\MineRust-data)"]
async fn window_v3_warmup_is_within_budget() {
    let env = match load_parity_env() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            return;
        }
    };
    let Some(config) = v3_config(&env) else {
        return;
    };
    let data = game_data(&env);

    // Build: level.dat + settings + tables + the loader pool (the part of
    // startup before the port is bound).
    let build_started = Instant::now();
    let (loader, _world) = server::build_world(&config, data)
        .unwrap_or_else(|error| panic!("v3 world cannot be built: {error}"));
    let build = build_started.elapsed();
    let loader = loader.expect("a configured world_dir always yields a loader");

    // Warm-up: the ready batch read once (what `server::run` does before it
    // binds).
    let warm_started = Instant::now();
    let reply = loader.load_batch().await.expect("warm-up can be queued");
    let ready = reply
        .await
        .expect("the warm-up reply channel is alive")
        .expect("the v3 world can be read");
    let warm = warm_started.elapsed();
    let total = build + warm;

    // Cache bytes: the encoded payload sizes of every ready chunk.
    let mut cache_bytes = 0_u64;
    for chunk in ready.chunks() {
        cache_bytes += chunk.encode_payload().expect("encodable chunk").len() as u64;
    }
    let ready_chunks = ready.len();

    let rss = working_set_bytes();
    let debug = cfg!(debug_assertions);
    eprintln!(
        "v3 warm-up ({}): build {:.1} ms, read {:.1} ms, total {:.1} ms; \
         ready {ready_chunks} chunks; cache {:.1} MB ({cache_bytes} B); \
         working set {}",
        if debug {
            "debug, unoptimized"
        } else {
            "release"
        },
        build.as_secs_f64() * 1000.0,
        warm.as_secs_f64() * 1000.0,
        total.as_secs_f64() * 1000.0,
        cache_bytes as f64 / (1024.0 * 1024.0),
        match rss {
            Some(bytes) => format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)),
            None => "не установлено (powershell unavailable)".to_owned(),
        }
    );

    // The owner's stop conditions judge the production (release) warm-up;
    // the debug build is the same code unoptimized, so its budget is the
    // same number relaxed by the factor the reader was measured to lose.
    let debug_factor = if debug { 6 } else { 1 };
    assert!(
        total <= WARM_BUDGET * debug_factor,
        "warm-up {total:?} exceeds the {WARM_BUDGET:?} budget (×{debug_factor})"
    );
    assert!(
        cache_bytes <= CACHE_BUDGET_BYTES,
        "cache {cache_bytes} B exceeds the {CACHE_BUDGET_BYTES} B budget"
    );
}

/// ШАГ 4.2–3: the П4/П5 walks on v3 against our in-process server, with the
/// participant-neutral spectator check off and tick metrics read from the
/// caller-owned handle.
#[tokio::test]
#[ignore = "requires the owner's installation (C:\\MineRust-data)"]
async fn window_v3_walk_matches_vanilla() {
    let env = match load_parity_env() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            return;
        }
    };
    let Some(config) = v3_config(&env) else {
        return;
    };
    if let Err(error) = std::fs::create_dir_all(TRACE_DIR) {
        eprintln!("SKIP: cannot create {TRACE_DIR}: {error}");
        return;
    }
    let ids = Arc::new(env.packet_ids().expect("packet ids"));

    let metrics = Arc::new(net::game::TickMetricsHandle::default());
    let (addr, abort) = start_with_world(&config, Arc::clone(&ids), Arc::clone(&metrics)).await;
    let _guard = ServerGuard { abort: Some(abort) };

    // П4 straight +x, then П5 diagonal +x/+z — the very routes of the
    // vanilla traces the walk is compared against.
    for (name, diagonal) in [("p4", false), ("p5", true)] {
        let trace = Path::new(TRACE_DIR).join(format!("window-{name}-ours.jsonl"));
        let _ = std::fs::remove_file(&trace);
        let mode = WalkMode::WalkV3 {
            diagonal,
            steps: STEPS,
            settle: SETTLE,
            spectator: false,
        };
        let stats =
            match run_walk_view_at(&env, Arc::clone(&ids), addr.port(), 10, mode, &trace).await {
                Ok(stats) => stats,
                Err(error) => panic!("{name}: walk failed: {error}"),
            };
        let snapshot = metrics.snapshot();
        eprintln!(
            "{name}: stop={:?} chunks={} moves={} teleports={} game_mode={:?} \
             abilities={:?}\n\
             {name}: tick duration p50/p99/max = {:.3}/{:.3}/{:.3} ms; \
             jitter p50/p99/max = {:.3}/{:.3}/{:.3} ms; late_ticks={} late_max={:.3} ms; \
             drift={:.3} ms (cumulative over the session)",
            stats.stop,
            stats.chunks,
            stats.moves,
            stats.teleports,
            stats.game_mode,
            stats.abilities_flags,
            snapshot.duration_p50_ms,
            snapshot.duration_p99_ms,
            snapshot.duration_max_ms,
            snapshot.jitter_p50_ms,
            snapshot.jitter_p99_ms,
            snapshot.jitter_max_ms,
            snapshot.late_ticks,
            snapshot.late_max_ms,
            snapshot.drift_ms,
        );
        assert!(
            matches!(stats.stop, Stop::Completed),
            "{name}: the walk must complete, not end on {:?}",
            stats.stop
        );
    }
}

/// Starts our own server in-process on the v3 world at an ephemeral port,
/// publishing its tick metrics into `metrics`.
async fn start_with_world(
    config: &server::ServerConfig,
    ids: Arc<data::PacketIds>,
    metrics: Arc<net::game::TickMetricsHandle>,
) -> (SocketAddr, tokio::task::AbortHandle) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr = listener.local_addr().expect("local addr");
    let status = Arc::new(server::status_info(config));
    let data = Arc::new(
        data::load_from_jar(
            &config.client_jar,
            config.registries_report.as_deref().map(Path::new),
        )
        .expect("game data from the jar"),
    );
    let (loader, world) = server::build_world(config, Arc::clone(&data))
        .unwrap_or_else(|error| panic!("v3 world cannot be built: {error}"));
    // Mirror `server::run`: the shared ready world is read once **before**
    // the port is used. Without this the first join triggers the whole v3
    // read inline (~13 s), the walking bot crosses several chunks behind
    // that read and its early window moves are dropped on purpose (only the
    // latest centre survives until the window exists) — a test artefact,
    // not the streaming defect the production warm-up rules out.
    if let Some(loader) = &loader {
        let reply = loader
            .load_batch()
            .await
            .expect("the v3 warm-up can be queued");
        reply
            .await
            .expect("the warm-up reply channel is alive")
            .expect("the v3 world can be read");
    }
    let game_config = net::game::GameConfig {
        workers: config.workers,
        region_side: config.region_side,
        ..net::game::GameConfig::default()
    };
    let server = tokio::spawn(net::serve_with_metrics(
        game_config,
        Some(metrics),
        listener,
        status,
        data,
        ids,
        loader,
        world,
    ));
    (addr, server.abort_handle())
}

/// Aborts the in-process server on every exit path, including a panic.
struct ServerGuard {
    /// Handle of the spawned `net::serve_with_metrics` task.
    abort: Option<tokio::task::AbortHandle>,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(abort) = self.abort.take() {
            abort.abort();
        }
    }
}
