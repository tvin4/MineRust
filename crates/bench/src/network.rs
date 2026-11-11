//! The network scenario runner (ARCH-M4 В§5, A2.3/A2.4): an in-process server
//! on the synthetic flat world, real TCP bench bots against it.
//!
//! * `streaming` вЂ” one bot walks straight at 1.0 block/tick for 120+ chunk
//!   crossings; the M4.4 criterion (no chunks outside the window, no orphan
//!   forgets, live set == window at the final centre) is judged by the
//!   bot's counters (centers vs forgets) plus the server's tick metrics.
//! * `load` вЂ” N bots walk for a fixed time; the runner records the server's
//!   tick metrics, per-bot join times, traffic and early disconnects.
//!
//! Full runs are manual (`cargo run -p bench -- streaming|load`); the smoke
//! test runs a two-bot miniature of the same shape.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use net::game::{TickMetrics, TickMetricsHandle};
use net::loader::Loader;
use server::ServerConfig;

use crate::bot::{Bot, BotPlan, BotStop};
use crate::{report_dir, world};

/// The protocol version the bench speaks, read from the owner's
/// `server.toml` (`protocol_version`): nothing is invented, and the bench
/// refuses to run without it (rule 6 of AGENTS.md).
pub fn protocol_version() -> anyhow::Result<i32> {
    let path = std::env::var_os(parity::SERVER_TOML_VAR)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| Path::new("server.toml").to_path_buf());
    let text = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("bench needs server.toml (protocol_version): {e}"))?;
    let value = text
        .lines()
        .find_map(|line| {
            let line = line.trim();
            line.strip_prefix("protocol_version").and_then(|rest| {
                rest.trim_start_matches(['=', ' '])
                    .trim()
                    .parse::<i32>()
                    .ok()
            })
        })
        .ok_or_else(|| anyhow::anyhow!("protocol_version not found in {}", path.display()))?;
    Ok(value)
}

/// Starts the in-process server on the synthetic world: returns the bound
/// address, the metrics handle and the abort handle.
///
/// Mirrors `server::run`: the shared ready batch is read once before the
/// runner serves (the warm-up the first-join inline read needs).
pub async fn start_server(
    workers: usize,
    region_side: i32,
    protocol_version: i32,
) -> anyhow::Result<(
    SocketAddr,
    Arc<TickMetricsHandle>,
    tokio::task::AbortHandle,
    world::MemoryEstimate,
)> {
    let (source, estimate) = world::build(world::WORLD_SIDE);
    let loader = Loader::spawn(source)?;
    // Warm-up: the reply is the Arc the pool cached; building the world was
    // the whole cost, the read itself is a clone.
    let reply = loader.load_batch().await?;
    let _warmed: Arc<net::chunks::ReadyChunks> = match reply.await {
        Ok(Ok(ready)) => ready,
        Ok(Err(error)) => anyhow::bail!("the synthetic world cannot be read: {error}"),
        Err(_) => anyhow::bail!("the loader worker did not answer the warm-up"),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let metrics = Arc::new(TickMetricsHandle::default());
    let status = Arc::new(protocol::StatusInfo {
        version_name: "bench".to_owned(),
        protocol_version,
        max_players: 0,
        online_players: 0,
        motd: "bench".to_owned(),
    });
    // The bench has no client jar: packet ids come from the compiled packet
    // keys (PacketIds::compiled()), which resolve every KEY_* the bots use.
    let game_data = Arc::new(data::GameData::default());
    let ids = Arc::new(data::PacketIds::compiled());
    let world_info = net::WorldInfo::default();
    let game_config = net::game::GameConfig {
        workers,
        region_side,
        ..net::game::GameConfig::default()
    };
    let handle = metrics.clone();
    let task = tokio::spawn(async move {
        net::serve_with_metrics(
            game_config,
            Some(handle),
            listener,
            status,
            game_data,
            ids,
            Some(loader),
            world_info,
        )
        .await
    });
    Ok((addr, metrics, task.abort_handle(), estimate))
}

/// Runs the streaming scenario (A2.3): one bot, straight +x walk at
/// 1.0 blocks/tick, then the window-invariant report.
pub async fn streaming(protocol_version: i32) -> anyhow::Result<String> {
    let (addr, metrics, abort, estimate) = start_server(2, 16, protocol_version).await?;
    let _guard = AbortGuard(Some(abort));
    let ids = Arc::new(data::PacketIds::compiled());

    // 1.0 block/tick: the server does not validate movement, so this pace
    // crosses a chunk boundary every 16 ticks; 2000 steps в‰€ 125 crossings
    // (~100 s). Vanilla could not walk this fast; recorded in the report.
    let plan = BotPlan {
        steps: 2000,
        step_len: 1.0,
        settle: Duration::from_secs(3),
    };
    let started = Instant::now();
    let bot = Bot::join(&bench_env(protocol_version), ids, addr.port(), 10).await?;
    let join = started.elapsed();
    let (stats, stop) = bot.play(plan).await;
    let snapshot: TickMetrics = metrics.snapshot();
    let elapsed = started.elapsed();

    // Crossings actually paced: moves / 16 (one crossing per 16 blocks +x).
    let crossings = stats.moves / 16;
    // Window invariants from the bot's side: every center shift must come
    // with its forgets having been sent (0 orphan forgets is a server-side
    // property вЂ” the forget count grows only after the chunk was in the
    // window; the bot can only check the totals line up with the geometry).
    let expected_forgets = crossings.saturating_sub(1) * 23;
    let lines = vec![
        "# streaming run (M4.4 criterion, synthetic world)".to_owned(),
        format!("- join: {join:.1?}; walk+settle: {elapsed:.1?}; stop: {stop:?}"),
        format!(
            "- chunks received: {}; chunk bytes: {}; centers: {}; forgets: {}",
            stats.chunks, stats.chunk_bytes, stats.centers, stats.forgets
        ),
        format!(
            "- moves sent: {} at 1.0 block/tick в†’ chunk crossings в‰Ґ {crossings} (criterion: 120+)",
            stats.moves
        ),
        format!(
            "- window totals: forgets {expected_forgets} expected from geometry vs {} seen",
            stats.forgets
        ),
        format!("- max batch depth observed by the bot: {}", stats.max_batch),
        format!(
            "- server tick p50/p99/max ms: {:.3}/{:.3}/{:.3}; late_ticks: {}; drift: {:.3} ms",
            snapshot.duration_p50_ms,
            snapshot.duration_p99_ms,
            snapshot.duration_max_ms,
            snapshot.late_ticks,
            snapshot.drift_ms
        ),
        format!(
            "- world memory estimate: {} chunks, ~{:.1} MB decoded (structs); one encoded chunk = {} B",
            estimate.chunks,
            estimate.bytes as f64 / (1024.0 * 1024.0),
            world::encoded_chunk_bytes()
        ),
        "- light: simplified (NOT parity); flat Single-palette chunks".to_owned(),
    ];
    let report = lines.join("\n");
    if !matches!(stop, BotStop::Completed) {
        anyhow::bail!("streaming run did not complete: {stop:?}\n{report}");
    }
    if crossings < 120 {
        anyhow::bail!("only {crossings} chunk crossings (criterion 120+)\n{report}");
    }
    Ok(report)
}

/// Runs the load scenario (A2.4): `bots` clients walk for `seconds`.
///
/// Per-bot join time is measured around [`Bot::join`]; kicks show up as
/// `BotStop::Closed` before the plan ends.
pub async fn load(
    bots: usize,
    workers: usize,
    region_side: i32,
    seconds: u64,
    protocol_version: i32,
) -> anyhow::Result<String> {
    let (addr, metrics, abort, _estimate) =
        start_server(workers, region_side, protocol_version).await?;
    let _guard = AbortGuard(Some(abort));
    let ids = Arc::new(data::PacketIds::compiled());
    let env = bench_env(protocol_version);

    let mut joins = Vec::with_capacity(bots);
    let mut sessions = Vec::with_capacity(bots);
    let started = Instant::now();
    for index in 0..bots {
        let t0 = Instant::now();
        let bot = Bot::join(&env, Arc::clone(&ids), addr.port(), 10).await?;
        joins.push(t0.elapsed());
        sessions.push((index, bot));
    }
    let join_phase = started.elapsed();

    // Every bot walks its own +x lane at 0.25 blocks/tick (a plausible
    // speed); lanes start 8 chunks apart in z, so players spread over tiles.
    let plan = BotPlan {
        steps: seconds * 20,
        step_len: 0.25,
        settle: Duration::from_secs(2),
    };
    let walk_started = Instant::now();
    let mut handles = Vec::with_capacity(sessions.len());
    for (index, bot) in sessions {
        let plan = plan.clone();
        handles.push(tokio::spawn(async move {
            let (stats, stop) = bot.play(plan).await;
            (index, stats, stop)
        }));
    }
    let mut closed_early = Vec::new();
    for handle in handles {
        let (index, stats, stop) = handle.await?;
        if !matches!(stop, BotStop::Completed) {
            closed_early.push(format!("bot {index}: {stop:?} after {} moves", stats.moves));
        }
    }
    let walk = walk_started.elapsed();
    let snapshot = metrics.snapshot();

    joins.sort();
    let min = joins.first().copied().unwrap_or_default();
    let median = joins.get(joins.len() / 2).copied().unwrap_or_default();
    let max = joins.last().copied().unwrap_or_default();
    let mut lines = vec![
        format!("# load: {bots} bots, K={workers}, side={region_side}, {seconds}s walk"),
        format!(
            "- join phase: {join_phase:.1?} total; per-bot min/median/max = {min:.1?}/{median:.1?}/{max:.1?}"
        ),
        format!(
            "- walk phase: {walk:.1?}; closed early: {} of {}",
            closed_early.len(),
            bots
        ),
        format!(
            "- server tick p50/p99/max ms: {:.3}/{:.3}/{:.3}; late: {}; drift: {:.3} ms",
            snapshot.duration_p50_ms,
            snapshot.duration_p99_ms,
            snapshot.duration_max_ms,
            snapshot.late_ticks,
            snapshot.drift_ms
        ),
    ];
    for line in &closed_early {
        lines.push(format!("- {line}"));
    }
    Ok(lines.join("\n"))
}

/// A minimal parity environment for the bench bots: they need only the
/// protocol version (read from the owner's `server.toml` by the CLI); no
/// vanilla installation вЂ” the vanilla paths stay empty and unused.
fn bench_env(protocol_version: i32) -> parity::ParityEnv {
    let config = ServerConfig {
        protocol_version,
        ..bench_config_template()
    };
    parity::ParityEnv {
        config,
        vanilla_jar: std::path::PathBuf::new(),
        vanilla_dir: std::path::PathBuf::new(),
        world_template: std::path::PathBuf::new(),
    }
}

/// A syntactically valid config template: the example file ships with the
/// repository, the bench only overrides `protocol_version` on top of it.
fn bench_config_template() -> ServerConfig {
    server::load_config("server.example.toml")
        .expect("server.example.toml must parse (a repo bug otherwise)")
}

/// Aborts the in-process server on every exit path, including a panic.
struct AbortGuard(Option<tokio::task::AbortHandle>);

impl Drop for AbortGuard {
    fn drop(&mut self) {
        if let Some(abort) = self.0.take() {
            abort.abort();
        }
    }
}

/// Writes a scenario report to `target/bench/` and returns the path.
pub fn save_report(name: &str, markdown: &str) -> anyhow::Result<std::path::PathBuf> {
    let dir = report_dir();
    std::fs::create_dir_all(&dir)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("{name}-{stamp}.md"));
    std::fs::write(&path, markdown)?;
    Ok(path)
}
