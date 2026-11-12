//! The network scenario runner (ARCH-M4 Р’В§5, A2.3/A2.4): an in-process server
//! on the synthetic flat world, real TCP bench bots against it.
//!
//! * `streaming` РІР‚вЂќ one bot walks straight at 1.0 block/tick for 120+ chunk
//!   crossings; the M4.4 criterion (no chunks outside the window, no orphan
//!   forgets, live set == window at the final centre) is judged by the
//!   bot's counters (centers vs forgets) plus the server's tick metrics.
//! * `load` РІР‚вЂќ N bots walk for a fixed time; the runner records the server's
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
use crate::cpu;
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
    tick_thread_priority: bool,
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
        tick_thread_priority,
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
    let (addr, metrics, abort, estimate) = start_server(2, 16, protocol_version, false).await?;
    let _guard = AbortGuard(Some(abort));
    let ids = Arc::new(data::PacketIds::compiled());

    // 1.0 block/tick: the server does not validate movement, so this pace
    // crosses a chunk boundary every 16 ticks; 2000 steps РІвЂ°в‚¬ 125 crossings
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
    // The M4.4 window invariant, from the bot's side: the live set the bot
    // holds at the end (closed batches minus forgets) must equal
    // window(final centre) at the announced view distance 10.
    let expected: std::collections::BTreeSet<(i32, i32)> = match stats.final_center {
        Some((cx, cz)) => net::window(cx, cz, 10).into_iter().collect(),
        None => std::collections::BTreeSet::new(),
    };
    let live: std::collections::BTreeSet<(i32, i32)> = stats.live.iter().copied().collect();
    let outside: Vec<(i32, i32)> = live.difference(&expected).copied().collect();
    let missing: Vec<(i32, i32)> = expected.difference(&live).copied().collect();
    let lines = vec![
        "# streaming run (M4.4 criterion, synthetic world)".to_owned(),
        format!("- join: {join:.1?}; walk+settle: {elapsed:.1?}; stop: {stop:?}"),
        format!(
            "- chunks received: {}; batches: {}; centers: {}; forgets: {}; bytes: {}",
            stats.chunks, stats.batches, stats.centers, stats.forgets, stats.chunk_bytes
        ),
        format!(
            "- moves sent: {} at 1.0 block/tick; chunk crossings = {crossings} (criterion: 120+)",
            stats.moves
        ),
        format!(
            "- final centre: {:?}; live set = {}; window(center, R=10) = {}",
            stats.final_center,
            live.len(),
            expected.len()
        ),
        format!(
            "- INVARIANTS: chunks outside window = {} (criterion 0); missing from window = {} (bot-side bookkeeping, see the report); orphan forgets are the server's debug_assert (M4.4 B)",
            outside.len(),
            missing.len()
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
    // Diagnostics when the run was traced: the first events of the log.
    let traced = if stats.event_log.is_empty() {
        String::new()
    } else {
        // The whole log goes to the report file (not committed); the console
        // keeps head+tail so a failure stays readable.
        let all = stats.event_log.join("\n");
        let head: Vec<String> = stats.event_log.iter().take(120).cloned().collect();
        let tail: Vec<String> = stats
            .event_log
            .iter()
            .rev()
            .take(120)
            .rev()
            .cloned()
            .collect();
        format!(
            "\n\n## event log ({} events; console shows head+tail, the file has all)\n```\n{}\n...\n{}\n```\n\n<details><summary>full log</summary>\n\n```\n{all}\n```\n</details>",
            stats.event_log.len(),
            head.join("\n"),
            tail.join("\n")
        )
    };
    if !matches!(stop, BotStop::Completed) {
        anyhow::bail!("streaming run did not complete: {stop:?}\n{report}{traced}");
    }
    if crossings < 120 {
        anyhow::bail!("only {crossings} chunk crossings (criterion 120+)\n{report}{traced}");
    }
    if !outside.is_empty() {
        anyhow::bail!(
            "{} chunks outside the window at the final centre (criterion 0): {:?}\n{report}{traced}",
            outside.len(),
            &outside[..outside.len().min(10)]
        );
    }
    let _ = save_report("streaming", &format!("{report}{traced}"))?;
    Ok(format!("{report}{traced}"))
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
        start_server(workers, region_side, protocol_version, false).await?;
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

/// One phase's tick metrics plus the system CPU busy share over that phase.
#[derive(Debug, Clone, Copy)]
pub struct PhaseMetrics {
    /// Median tick duration, ms.
    pub tick_p50_ms: f64,
    /// 99th percentile tick duration, ms.
    pub tick_p99_ms: f64,
    /// Worst tick duration, ms.
    pub tick_max_ms: f64,
    /// Ticks that woke more than 20 ms late.
    pub late_ticks: u64,
    /// System-wide CPU busy share over the phase, 0..=100 (`None` when the
    /// platform has no CPU counters or the share was not measured).
    pub cpu_busy_percent: Option<f64>,
}

impl PhaseMetrics {
    fn from_snapshot(snapshot: TickMetrics, cpu_busy_percent: Option<f64>) -> Self {
        Self {
            tick_p50_ms: snapshot.duration_p50_ms,
            tick_p99_ms: snapshot.duration_p99_ms,
            tick_max_ms: snapshot.duration_max_ms,
            late_ticks: snapshot.late_ticks,
            cpu_busy_percent,
        }
    }
}

/// Runs the M4.5 Шаг 0.2 split (BENCH-M4 §5): one join phase, then a steady
/// walking phase measured `warmup` after the last bot joined.
///
/// The join phase is everything the server ticks while the `bots` clients
/// join (measured through the cumulative metrics, which up to that point
/// describe only the join). The walking bots then send moves; the metrics are
/// reset at the start of walking and again after `warmup`, so the steady
/// phase covers the walking window only and never mixes in the join.
pub async fn load_phased(
    bots: usize,
    workers: usize,
    region_side: i32,
    warmup: Duration,
    measure: Duration,
    protocol_version: i32,
) -> anyhow::Result<(PhaseMetrics, PhaseMetrics)> {
    let (addr, metrics, abort, _estimate) =
        start_server(workers, region_side, protocol_version, false).await?;
    let _guard = AbortGuard(Some(abort));
    let ids = Arc::new(data::PacketIds::compiled());
    let env = bench_env(protocol_version);

    // --- join phase: everything up to the last bot's entry teleport ---
    let join_cpu_start = cpu::sample();
    let mut sessions = Vec::with_capacity(bots);
    for _ in 0..bots {
        sessions.push(Bot::join(&env, Arc::clone(&ids), addr.port(), 10).await?);
    }
    let join = PhaseMetrics::from_snapshot(
        metrics.snapshot(),
        join_cpu_start.and_then(|start| cpu::busy_percent(start, cpu::sample()?)),
    );

    // --- steady walking: 5 s warm-up discarded, then the measured window ---
    metrics.request_phase_reset();
    let seconds = (warmup + measure).as_secs();
    let plan = BotPlan {
        steps: seconds * 20,
        step_len: 0.25,
        settle: Duration::from_secs(1),
    };
    let mut handles = Vec::with_capacity(sessions.len());
    for bot in sessions {
        let plan = plan.clone();
        handles.push(tokio::spawn(async move { bot.play(plan).await }));
    }
    tokio::time::sleep(warmup).await;
    metrics.request_phase_reset();
    let steady_cpu_start = cpu::sample();
    for handle in handles {
        handle.await?;
    }
    let steady = PhaseMetrics::from_snapshot(
        metrics.snapshot(),
        steady_cpu_start.and_then(|start| cpu::busy_percent(start, cpu::sample()?)),
    );
    Ok((join, steady))
}

/// One-cell CPU percentage for a markdown table row.
fn cpu_cell(percent: Option<f64>) -> String {
    percent.map_or_else(|| "n/a".to_owned(), |p| format!("{p:.0}"))
}

/// Runs the Шаг 0.2 series: N=100/K=6/side=16 and N=200/K=10/side=16, three
/// repeats each, join phase vs steady walking, and writes the report.
pub async fn phase_cli(protocol_version: i32) -> anyhow::Result<String> {
    let warmup = Duration::from_secs(5);
    let measure = Duration::from_secs(30);
    let configs = [(100usize, 6usize, 16i32), (200, 10, 16)];
    let repeats = 3;
    let mut markdown = String::from(
        "# M4.5 Шаг 0.2 — join phase vs steady walking (BENCH-M4 §5)\n\n\
         join = all server ticks while the bots join; steady = walking, from \
         5 s after the last bot joined, 30 s measured (metrics reset at the \
         phase boundaries). CPU = system-wide busy share over the phase.\n\n\
         | N | K | side | repeat | join p50/p99/max ms | join late | join CPU % | steady p50/p99/max ms | steady late | steady CPU % |\n\
         |---|---|---|---|---|---|---|---|---|---|\n",
    );
    for (bots, workers, side) in configs {
        for repeat in 1..=repeats {
            let (join, steady) =
                load_phased(bots, workers, side, warmup, measure, protocol_version).await?;
            markdown.push_str(&format!(
                "| {bots} | {workers} | {side} | {repeat} | {:.3}/{:.3}/{:.3} | {} | {} | {:.3}/{:.3}/{:.3} | {} | {} |\n",
                join.tick_p50_ms,
                join.tick_p99_ms,
                join.tick_max_ms,
                join.late_ticks,
                cpu_cell(join.cpu_busy_percent),
                steady.tick_p50_ms,
                steady.tick_p99_ms,
                steady.tick_max_ms,
                steady.late_ticks,
                cpu_cell(steady.cpu_busy_percent),
            ));
            println!(
                "N={bots} K={workers} side={side} repeat {repeat}: \
                 join {:.3}/{:.3}/{:.3} ms late={} cpu={} | \
                 steady {:.3}/{:.3}/{:.3} ms late={} cpu={}",
                join.tick_p50_ms,
                join.tick_p99_ms,
                join.tick_max_ms,
                join.late_ticks,
                cpu_cell(join.cpu_busy_percent),
                steady.tick_p50_ms,
                steady.tick_p99_ms,
                steady.tick_max_ms,
                steady.late_ticks,
                cpu_cell(steady.cpu_busy_percent),
            );
        }
    }
    let path = save_report("m45-phase", &markdown)?;
    println!("\nreport: {}", path.display());
    Ok(markdown)
}

/// The A3 priority A/B scenario: 100 network bots walk (paced W1) while 8
/// background busy threads spin; the flag [`GameConfig::tick_thread_priority`]
/// is on for the B-half, off for the A-half. `repeats` per half, the runs are
/// sequential (a shared machine must not measure itself against itself).
///
/// The verdict rule: a difference is only reported when it exceeds the
/// spread inside the repeats; otherwise В«СЂР°Р·Р»РёС‡РёРµ РЅРµ СѓСЃС‚Р°РЅРѕРІР»РµРЅРѕВ».
pub async fn priority_ab(
    bots: usize,
    seconds: u64,
    repeats: u32,
    protocol_version: i32,
) -> anyhow::Result<String> {
    /// One busy background thread: pure CPU, no syscalls, black_boxed.
    fn spin_worker(stop: Arc<std::sync::atomic::AtomicBool>) {
        let mut acc = 0u64;
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            for _ in 0..10_000 {
                acc = acc.wrapping_mul(6364136223846793005).wrapping_add(1);
                std::hint::black_box(&acc);
            }
        }
    }

    let mut lines = vec![format!(
        "# priority A/B: {bots} bots, 8 busy threads, {seconds}s per run, {repeats} repeats per half"
    )];
    for half in [false, true] {
        let label = if half {
            "B (priority on)"
        } else {
            "A (priority off)"
        };
        for repeat in 1..=repeats {
            let (addr, metrics, abort, _estimate) =
                start_server(2, 16, protocol_version, half).await?;
            let _guard = AbortGuard(Some(abort));
            let ids = Arc::new(data::PacketIds::compiled());
            let env = bench_env(protocol_version);

            // Background busy threads: started just before the bots.
            let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let mut spinners = Vec::with_capacity(8);
            for _ in 0..8 {
                let flag = Arc::clone(&stop_flag);
                spinners.push(std::thread::spawn(move || spin_worker(flag)));
            }

            let mut sessions = Vec::with_capacity(bots);
            for _ in 0..bots {
                let bot = Bot::join(&env, Arc::clone(&ids), addr.port(), 10).await?;
                sessions.push(bot);
            }
            let plan = BotPlan {
                steps: seconds * 20,
                step_len: 0.25,
                settle: Duration::from_secs(2),
            };
            let mut handles = Vec::with_capacity(sessions.len());
            for bot in sessions {
                let plan = plan.clone();
                handles.push(tokio::spawn(async move { bot.play(plan).await }));
            }
            for handle in handles {
                handle.await?;
            }
            stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);
            for spinner in spinners {
                let _ = spinner.join();
            }
            let snapshot = metrics.snapshot();
            lines.push(format!(
                "- {label} repeat {repeat}: tick p50/p99/max ms = {:.3}/{:.3}/{:.3}; late: {}; drift: {:.3} ms",
                snapshot.duration_p50_ms,
                snapshot.duration_p99_ms,
                snapshot.duration_max_ms,
                snapshot.late_ticks,
                snapshot.drift_ms
            ));
        }
    }
    Ok(lines.join("\n"))
}

/// A minimal parity environment for the bench bots: they need only the
/// protocol version (read from the owner's `server.toml` by the CLI); no
/// vanilla installation РІР‚вЂќ the vanilla paths stay empty and unused.
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
