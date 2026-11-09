//! M4.0 (а): the walk-trace bot — three runs, each on a fresh copy of the
//! frozen v2 template (P1 stand 10 s, P2 straight along +x, P3 diagonal
//! +x/−z), driven with the owner's fixed stop conditions (see
//! `docs/ARCH-M4.md`, §6 «M4.0 (а)»): tuned nothing, stopped at the first
//! violation and reported what exists.
//!
//! Every run hashes the world copy before, stops with `save-all flush` +
//! `stop`, hashes after and classifies the diff: any changed `region/*.mca`
//! aborts the sequence, `level.dat`/`playerdata`/`session.lock` are
//! expected noise, `entities/` and `poi/` are listed separately. The
//! server's gamemode is probed with `execute ... run say ...` markers read
//! back from `logs/latest.log` while the bot is still online.
//!
//! Traces (JSONL, monotonic time, base64 payloads) and per-run reports land
//! in `C:\MineRust-data\traces\`; the repository keeps only this source.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use parity::{load_parity_env, run_walk, ParityEnv, RunStats, Stop, Vanilla, WalkMode};
use sha2::{Digest, Sha256};

/// Where traces and reports land (outside the repository, by design).
const TRACE_DIR: &str = r"C:\MineRust-data\traces";
/// P1: seconds idling on place inside play.
const P1_HOLD: Duration = Duration::from_secs(10);
/// P2/P3: movement packets sent (each step ≤ 0.2 blocks).
const WALK_STEPS: u32 = 200;
/// P2/P3: idle seconds after the last step, then quit.
const SETTLE: Duration = Duration::from_secs(5);
/// Interval between the server-side gamemode probes.
const CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// How many probes may be sent while the bot is still playing.
const CHECK_ATTEMPTS: u32 = 8;
/// `say` marker each probe writes into the log when the bot is spectator.
const GM_SPECTATOR: &str = "WALK_GM_SPECTATOR";
/// `say` marker for «the server is not in spectator» — a hard stop.
const GM_NOT_SPECTATOR: &str = "WALK_GM_NOT_SPECTATOR";

/// Reads the parity environment or explains why the test is skipped.
fn parity_env_or_skip() -> Option<ParityEnv> {
    match load_parity_env() {
        Ok(env) => Some(env),
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            None
        }
    }
}

/// SHA-256 of every file under `dir`, keyed by the relative path (`/`).
fn hash_tree(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if !dir.is_dir() {
        return out;
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(&current).expect("read world tree") {
            let entry = entry.expect("world entry");
            let path = entry.path();
            if entry.file_type().expect("entry type").is_dir() {
                stack.push(path);
                continue;
            }
            let rel = path
                .strip_prefix(dir)
                .expect("relative path")
                .to_string_lossy()
                .replace('\\', "/");
            let mut hasher = Sha256::new();
            hasher.update(std::fs::read(&path).expect("read world file"));
            let hex: String = hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            out.insert(rel, hex);
        }
    }
    out
}

/// Bucket of one changed world file: the owner's categories.
fn bucket(rel: &str) -> &'static str {
    let p = rel.replace('\\', "/");
    let nested = |key: &str| p.starts_with(key) || p.contains(&format!("/{key}/"));
    if nested("region") {
        "region"
    } else if nested("entities") {
        "entities"
    } else if nested("poi") {
        "poi"
    } else if nested("playerdata")
        || p == "level.dat"
        || p.ends_with("/level.dat")
        || p == "session.lock"
        || p.ends_with("/session.lock")
    {
        "expected"
    } else {
        "other"
    }
}

/// The classified SHA-256 diff of one run's world copy.
#[derive(Debug, Default)]
struct Diff {
    /// Changed `region/*.mca` — a hard stop for the whole sequence.
    region: Vec<String>,
    /// Changed `entities/` files (listed separately, per the owner).
    entities: Vec<String>,
    /// Changed `poi/` files (listed separately, per the owner).
    poi: Vec<String>,
    /// `level.dat` / `playerdata/` / `session.lock` — expected noise.
    expected: Vec<String>,
    /// Everything else (listed, not a stop by itself).
    other: Vec<String>,
}

impl Diff {
    /// Whether any `region/*.mca` changed.
    fn region_dirty(&self) -> bool {
        !self.region.is_empty()
    }
}

/// Compares two snapshots of the same tree (added/changed/removed count).
fn diff(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Diff {
    let mut d = Diff::default();
    let mut note = |rel: &str, what: &str| {
        let line = format!("{rel} ({what})");
        match bucket(rel) {
            "region" => d.region.push(line),
            "entities" => d.entities.push(line),
            "poi" => d.poi.push(line),
            "expected" => d.expected.push(line),
            _ => d.other.push(line),
        }
    };
    for (rel, old) in before {
        match after.get(rel) {
            Some(new) if new == old => {}
            Some(_) => note(rel, "changed"),
            None => note(rel, "removed"),
        }
    }
    for rel in after.keys() {
        if !before.contains_key(rel) {
            note(rel, "added");
        }
    }
    d
}

/// What the log probes could prove about the server's gamemode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GmStatus {
    /// The spectator marker fired: the server forces spectator.
    Spectator,
    /// The «not spectator» marker fired: a hard stop.
    NotSpectator,
    /// No marker arrived; the report carries the raw evidence lines.
    Unverified,
}

/// Everything one run produced, for the summary and the owner's report.
struct RunOutcome {
    /// `walk-p1` / `walk-p2` / `walk-p3`.
    name: &'static str,
    /// Human description of the mode.
    mode: String,
    /// JSONL trace path.
    trace: PathBuf,
    /// Per-run report path.
    report: PathBuf,
    /// Bot statistics, `None` when the run failed before play.
    stats: Option<RunStats>,
    /// Fatal error (connect/join/stop), if any.
    fatal: Option<String>,
    /// Classified world diff of the copy.
    diff: Diff,
    /// Gamemode verdict read from the server log.
    gm: GmStatus,
    /// The owner's stop conditions found by this run.
    violations: Vec<String>,
}

/// Runs one walk on a fresh copy: start → hash → play → flush stop →
/// hash → diff → log probes → report file. Never panics: a dead server
/// becomes a violation like any other stop condition.
async fn one_run(
    env: &ParityEnv,
    ids: Arc<PacketIds>,
    name: &'static str,
    mode: WalkMode,
    first_run: bool,
) -> RunOutcome {
    let trace = Path::new(TRACE_DIR).join(format!("{name}.jsonl"));
    let report = Path::new(TRACE_DIR).join(format!("{name}.report.txt"));
    let mode_label = match mode {
        WalkMode::Stand { hold } => format!("stand {:.0}s", hold.as_secs()),
        WalkMode::Straight { steps, .. } => format!("straight {steps} steps"),
        WalkMode::Diagonal { steps, .. } => format!("diagonal {steps} steps"),
    };
    let mut outcome = RunOutcome {
        name,
        mode: mode_label,
        trace,
        report,
        stats: None,
        fatal: None,
        diff: Diff::default(),
        gm: GmStatus::Unverified,
        violations: Vec::new(),
    };

    let vanilla = match Vanilla::start(env).await {
        Ok(server) => server,
        Err(error) => {
            let text = format!("vanilla start: {error}");
            outcome.fatal = Some(text.clone());
            outcome.violations.push(text);
            write_report(&outcome, &[]);
            return outcome;
        }
    };
    let log_path = vanilla.log_path().to_path_buf();
    let world = env.vanilla_dir.join("world");
    let before = hash_tree(&world);

    // The bot plays while the server future probes the gamemode and,
    // once the bot is done (`quit`), performs `save-all flush` + `stop`.
    let quit = Arc::new(AtomicBool::new(false));
    let mut probe_err: Option<String> = None;
    let (walk_res, stop_res) = {
        let walk_quit = Arc::clone(&quit);
        let mut server = vanilla;
        let walk = async {
            let res = run_walk(env, Arc::clone(&ids), mode, &outcome.trace).await;
            walk_quit.store(true, Ordering::SeqCst);
            res
        };
        let probes = async {
            for _ in 0..CHECK_ATTEMPTS {
                tokio::time::sleep(CHECK_INTERVAL).await;
                if quit.load(Ordering::SeqCst) {
                    break;
                }
                let spectator = server
                    .command(&format!(
                        "execute if entity ParityBot[gamemode=spectator] run say {GM_SPECTATOR}"
                    ))
                    .await;
                let not_spectator = server
                    .command(&format!(
                        "execute if entity ParityBot[gamemode=!spectator] run say \
                         {GM_NOT_SPECTATOR}"
                    ))
                    .await;
                if let Err(error) = spectator.and(not_spectator) {
                    probe_err = Some(error.to_string());
                    break;
                }
                if let Ok(text) = std::fs::read_to_string(&log_path) {
                    if text.contains(GM_SPECTATOR) || text.contains(GM_NOT_SPECTATOR) {
                        break;
                    }
                }
            }
            server.stop_with_flush().await
        };
        tokio::join!(walk, probes)
    };
    let after = hash_tree(&world);
    outcome.diff = diff(&before, &after);

    // Log verdict: both markers keep their raw lines as evidence.
    let log_text = std::fs::read_to_string(&log_path).unwrap_or_default();
    outcome.gm = if log_text.contains(GM_SPECTATOR) {
        GmStatus::Spectator
    } else if log_text.contains(GM_NOT_SPECTATOR) {
        GmStatus::NotSpectator
    } else {
        GmStatus::Unverified
    };
    let log_evidence: Vec<String> = log_text
        .lines()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("walk_gm") || lower.contains("gamemode") || lower.contains("invalid")
        })
        .map(ToOwned::to_owned)
        .take(40)
        .collect();

    // The owner's stop conditions, evaluated once with fixed criteria.
    match walk_res {
        Ok(stats) => {
            if stats.stop != Stop::Completed {
                outcome
                    .violations
                    .push(format!("stop reason: {:?}", stats.stop));
            }
            if first_run && stats.teleports != 1 {
                outcome.violations.push(format!(
                    "P1 server player_position = {} (must be exactly 1)",
                    stats.teleports
                ));
            }
            if !first_run && stats.teleports > 1 {
                outcome.violations.push(format!(
                    "server player_position after the first: {} (rollback)",
                    stats.teleports
                ));
            }
            outcome.stats = Some(stats);
        }
        Err(error) => {
            let text = format!("walk failed: {error}");
            outcome.fatal = Some(text.clone());
            outcome.violations.push(text);
        }
    }
    if let Some(error) = probe_err {
        outcome.violations.push(format!("gamemode probe: {error}"));
    }
    if let Err(error) = stop_res {
        outcome.violations.push(format!("server stop: {error}"));
    }
    if outcome.gm == GmStatus::NotSpectator {
        outcome
            .violations
            .push("server is not in spectator".to_owned());
    }
    if outcome.diff.region_dirty() {
        outcome.violations.push(format!(
            "region/*.mca changed: {}",
            outcome.diff.region.join(", ")
        ));
    }
    if outcome.gm == GmStatus::Unverified {
        // Not one of the owner's stop triggers (nothing proved otherwise);
        // the report still carries it as an unverified fact.
        eprintln!("NOTE {name}: gamemode unverified; see the report's evidence");
    }

    write_report(&outcome, &log_evidence);
    outcome
}

/// Writes one run's report file (facts only, no conclusions).
fn write_report(outcome: &RunOutcome, evidence: &[String]) {
    let mut text = String::new();
    text.push_str(&format!(
        "# Walk trace {} ({})\n",
        outcome.name, outcome.mode
    ));
    text.push_str(&format!(
        "fatal: {}\n",
        outcome.fatal.as_deref().unwrap_or("none")
    ));
    match &outcome.stats {
        None => text.push_str("no play statistics (run failed before play)\n"),
        Some(stats) => {
            text.push_str(&format!(
                "stop: {:?}\nelapsed: {:.0} ms | server player_position: {} | \
                 movement packets sent: {:?}\nspawn: {:?} | last position: {:?}\n",
                stats.stop, stats.elapsed_ms, stats.teleports, stats.moves, stats.spawn, stats.last
            ));
            text.push_str("\n## Packet counts (direction / name: times)\n");
            for (key, value) in &stats.counts {
                text.push_str(&format!("{key}: {value}\n"));
            }
            text.push_str(
                "\n## Chunk batches (tick = t_ms / 50; timings possibly \
                 distorted by generation)\n",
            );
            if stats.batches.is_empty() {
                text.push_str("(no finished batch)\n");
            }
            for batch in &stats.batches {
                text.push_str(&format!(
                    "t={:.0} ms (tick {}: {} chunks, {} bytes)\n",
                    batch.t_ms,
                    (batch.t_ms / 50.0) as u64,
                    batch.chunks,
                    batch.bytes
                ));
            }
            text.push_str(
                "\n## Server window moves (tick = t_ms / 50; timings possibly \
                 distorted by generation)\n",
            );
            for mv in &stats.cache_moves {
                text.push_str(&format!(
                    "t={:.0} ms (tick {}) {} ({}, {})\n",
                    mv.t_ms,
                    (mv.t_ms / 50.0) as u64,
                    mv.name,
                    mv.x,
                    mv.z
                ));
            }
        }
    }
    text.push_str("\n## World diff after save-all flush + stop (SHA-256)\n");
    text.push_str(&format!("region: {}\n", joined(&outcome.diff.region)));
    text.push_str(&format!("entities: {}\n", joined(&outcome.diff.entities)));
    text.push_str(&format!("poi: {}\n", joined(&outcome.diff.poi)));
    text.push_str(&format!(
        "expected (level.dat/playerdata/session.lock): {}\n",
        joined(&outcome.diff.expected)
    ));
    text.push_str(&format!("other: {}\n", joined(&outcome.diff.other)));
    text.push_str("\n## Gamemode probes (logs/latest.log)\n");
    text.push_str(&format!("verdict: {:?}\n", outcome.gm));
    for line in evidence {
        text.push_str(line);
        text.push('\n');
    }
    text.push_str("\n## Stop conditions found\n");
    if outcome.violations.is_empty() {
        text.push_str("none\n");
    }
    for violation in &outcome.violations {
        text.push_str(violation);
        text.push('\n');
    }
    text.push_str("\n## Trace\n");
    text.push_str(&format!("JSONL: {}\n", outcome.trace.display()));

    if let Err(error) = std::fs::write(&outcome.report, text) {
        eprintln!("cannot write {}: {error}", outcome.report.display());
    }
}

/// `"(a, b, …)"` or `"none"` for the report's diff lines.
fn joined(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_owned()
    } else {
        items.join(", ")
    }
}

/// The owner's three fixed runs; the first violating run stops the rest.
#[tokio::test]
#[ignore = "requires the owner's vanilla installation (C:\\MineRust-data)"]
async fn walk_trace_three_runs() {
    let env = match parity_env_or_skip() {
        Some(env) => env,
        None => return,
    };
    let ids = Arc::new(env.packet_ids().expect("packet ids"));
    if let Err(error) = std::fs::create_dir_all(TRACE_DIR) {
        eprintln!("SKIP: cannot create {TRACE_DIR}: {error}");
        return;
    }
    let template_before = hash_tree(&env.world_template);

    let scripts: [(&'static str, WalkMode, bool); 3] = [
        ("walk-p1", WalkMode::Stand { hold: P1_HOLD }, true),
        (
            "walk-p2",
            WalkMode::Straight {
                steps: WALK_STEPS,
                settle: SETTLE,
            },
            false,
        ),
        (
            "walk-p3",
            WalkMode::Diagonal {
                steps: WALK_STEPS,
                settle: SETTLE,
            },
            false,
        ),
    ];
    let mut performed: Vec<RunOutcome> = Vec::new();
    let mut aborted_after: Option<&str> = None;
    for (name, mode, first_run) in scripts {
        if let Some(previous) = performed.last() {
            if !previous.violations.is_empty() {
                aborted_after = Some(previous.name);
                eprintln!("STOP after {}: {:?}", previous.name, previous.violations);
                break;
            }
        }
        let outcome = one_run(&env, Arc::clone(&ids), name, mode, first_run).await;
        eprintln!(
            "{}: stop={:?} teleports={} region={} gm={:?}",
            outcome.name,
            outcome
                .stats
                .as_ref()
                .map(|stats| format!("{:?}", stats.stop))
                .unwrap_or_else(|| "n/a".to_owned()),
            outcome.stats.as_ref().map_or(0, |s| s.teleports),
            joined(&outcome.diff.region),
            outcome.gm
        );
        performed.push(outcome);
    }

    // The frozen template must be byte-identical across all runs.
    let template_after = hash_tree(&env.world_template);
    let template_diff = diff(&template_before, &template_after);

    let summary_path = Path::new(TRACE_DIR).join("walk-summary.md");
    let mut summary = String::from("# M4.0 (а) walk traces — summary\n\n");
    for run in &performed {
        summary.push_str(&format!(
            "- {}: fatal={} stop-conditions={:?} report={} trace={}\n",
            run.name,
            run.fatal.is_some(),
            run.violations,
            run.report.display(),
            run.trace.display()
        ));
    }
    summary.push_str(&format!(
        "\ntemplate region changed ({} files): {}\n",
        template_diff.region.len(),
        joined(&template_diff.region)
    ));
    summary.push_str(&format!(
        "template other changes: {}\n",
        joined(&template_diff.other)
    ));
    summary.push_str(&format!(
        "\naborted after: {}\n",
        aborted_after.unwrap_or("nothing — all three runs completed")
    ));
    summary.push_str(
        "notes: chunk-batch timings are possibly distorted by generation; \
         pace 4.6715 depends on the bot; gamemode verdicts come from the \
         server log probes.\n",
    );
    if let Err(error) = std::fs::write(&summary_path, summary) {
        eprintln!("cannot write {}: {error}", summary_path.display());
    }
    eprintln!("summary: {}", summary_path.display());

    // Report everything, panic only on the hard invariants the owner set.
    let region_dirty =
        performed.iter().any(|run| run.diff.region_dirty()) || template_diff.region_dirty();
    assert!(
        !region_dirty,
        "STOP: region/*.mca changed — see walk-summary.md"
    );
    assert!(
        performed.iter().all(|run| run.gm != GmStatus::NotSpectator),
        "STOP: server is not in spectator — see walk-summary.md"
    );
}
