//! M4.5 step 7.2: vanilla two-bot runs with a **non-spectator** default game
//! mode, to obtain the game-mode-dependent bytes of the entity stream.
//!
//! The C1–C4 traces were recorded with `gamemode=spectator` (the bots were
//! spectators), so their `player_info_update`/`set_entity_data` bytes carry
//! spectator values (step 7.1). These runs repeat the C1 script — bot A at
//! spawn observing, bot B joining nearby, standing 10 s and leaving cleanly —
//! on a fresh copy of the frozen v3 template with the copy's
//! `server.properties` forced to:
//!
//! * C5 — `gamemode=survival` (login `game_mode` must prove 0);
//! * C6 — `gamemode=creative` (login `game_mode` must prove 1);
//! * `force-gamemode=true`, `view-distance=10`, `simulation-distance=10`.
//!
//! Bots stand still (the join script sends no movement), so the only packets
//! of interest are what vanilla sends about B to A. The frozen v3 template is
//! hashed before and after every run and must stay byte-identical. Traces
//! land under `C:\MineRust-data\traces` as `two-c{5,6}-{A,B}.jsonl`; the
//! byte analysis is `tools/trace-analyze/m45-step72-gamemode.py`.
//!
//! The bots do not enforce spectator (no `enforce_mode` in this driver — the
//! mode check here expects the *scenario's* mode instead).
//!
//! ```text
//! cargo test -p parity --release --test two_bot_gamemode -- --ignored --nocapture
//! ```

#[path = "common/twobot_scenarios.rs"]
#[allow(dead_code)]
mod scenarios;
#[path = "common/twobot.rs"]
#[allow(dead_code)]
mod twobot;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use parity::{load_parity_env, ParityEnv, Vanilla};
use scenarios::{BScript, Observer, Scene};
use sha2::{Digest, Sha256};
use twobot::{run_bot, BotReport, Shared, BOT_A, BOT_B};

/// Where traces and reports land (outside the repository, by design).
const TRACE_DIR: &str = r"C:\MineRust-data\traces";
/// The frozen v3 world template every run gets a fresh copy of.
const V3_TEMPLATE: &str = r"C:\MineRust-data\world-template-v3";
/// Budget for both bots to reach play.
const JOIN_WAIT: Duration = Duration::from_secs(180);
/// Per-bot play-loop budget (the C1 join script plus a margin).
const SAFETY: Duration = Duration::from_secs(180);

/// The two game-mode scenarios of step 7.2.
struct ModeRun {
    /// File-name code (`c5`/`c6`).
    code: &'static str,
    /// Human description of the run.
    desc: &'static str,
    /// `gamemode=` value for the copy's `server.properties`.
    gamemode: &'static str,
    /// `game_mode` the clientbound login (play) must prove.
    expected_mode: i32,
}

/// C5 (survival) and C6 (creative).
const RUNS: [ModeRun; 2] = [
    ModeRun {
        code: "c5",
        desc: "C5: gamemode=survival, B joins, stands 10 s, leaves",
        gamemode: "survival",
        expected_mode: 0,
    },
    ModeRun {
        code: "c6",
        desc: "C6: gamemode=creative, B joins, stands 10 s, leaves",
        gamemode: "creative",
        expected_mode: 1,
    },
];

/// Everything one run produced, for its report and the summary.
struct RunOutcome {
    /// The run descriptor.
    run: &'static ModeRun,
    /// Bot A's JSONL path.
    a_trace: PathBuf,
    /// Bot B's JSONL path.
    b_trace: PathBuf,
    /// The per-run report path.
    report: PathBuf,
    /// Bot A's final report (absent only if its task failed before play).
    a: Option<BotReport>,
    /// Bot B's final report (`None` when B was never started).
    b: Option<BotReport>,
    /// Report facts (script ends, proven modes).
    facts: Vec<String>,
    /// The owner's stop conditions found by this run.
    violations: Vec<String>,
    /// Files of the frozen template that changed during the run.
    template_changed: Vec<String>,
}

impl RunOutcome {
    /// Fresh outcome with the paths of one run.
    fn new(run: &'static ModeRun) -> Self {
        let dir = Path::new(TRACE_DIR);
        Self {
            run,
            a_trace: dir.join(format!("two-{}-A.jsonl", run.code)),
            b_trace: dir.join(format!("two-{}-B.jsonl", run.code)),
            report: dir.join(format!("two-{}.report.txt", run.code)),
            a: None,
            b: None,
            facts: Vec::new(),
            violations: Vec::new(),
            template_changed: Vec::new(),
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
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let Ok(relative) = path.strip_prefix(dir) else {
                continue;
            };
            let key = relative.to_string_lossy().replace('\\', "/");
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            out.insert(key, format!("{:x}", hasher.finalize()));
        }
    }
    out
}

/// Relative paths whose content changed, added or removed (`template`
/// must produce an empty list — the frozen copy source is read-only).
fn changed(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (rel, old) in before {
        match after.get(rel) {
            Some(new) if new == old => {}
            Some(_) => out.push(format!("{rel} (changed)")),
            None => out.push(format!("{rel} (removed)")),
        }
    }
    for rel in after.keys() {
        if !before.contains_key(rel) {
            out.push(format!("{rel} (added)"));
        }
    }
    out
}

/// Rewrites the four game-mode keys of the copy's `server.properties`,
/// leaving every other key byte for byte. The caller restores the file with
/// a guard (the owner's runs end with `gamemode=spectator`).
fn set_properties(env: &ParityEnv, gamemode: &str) -> Result<(), std::io::Error> {
    let path = env.vanilla_dir.join("server.properties");
    let original = std::fs::read_to_string(&path)?;
    let targets = [
        ("gamemode=", gamemode),
        ("force-gamemode=", "true"),
        ("view-distance=", "10"),
        ("simulation-distance=", "10"),
    ];
    let mut out = String::new();
    for line in original.lines() {
        let mut replaced = None;
        for (key, value) in targets {
            if line.starts_with(key) {
                replaced = Some(format!("{key}{value}"));
                break;
            }
        }
        out.push_str(replaced.as_deref().unwrap_or(line));
        out.push('\n');
    }
    std::fs::write(&path, &out)?;
    Ok(())
}

/// Restores the copy's `server.properties` on every exit path, panic too.
struct PropertiesGuard {
    /// Path of the file to restore.
    path: PathBuf,
    /// The original text.
    original: String,
}

impl Drop for PropertiesGuard {
    fn drop(&mut self) {
        if let Err(error) = std::fs::write(&self.path, &self.original) {
            eprintln!("cannot restore {}: {error}", self.path.display());
        }
    }
}

/// One-line description of the login `game_mode` fact of one bot.
fn mode_fact(side: &str, run: &ModeRun, report: &BotReport) -> Option<String> {
    if report.game_mode == Some(run.expected_mode) {
        Some(format!(
            "{side}: login game_mode {:?} (expected {}, {})",
            report.game_mode, run.expected_mode, run.gamemode
        ))
    } else {
        None
    }
}

/// The mode mismatch is a stop condition like any other.
fn mode_problem(side: &str, run: &ModeRun, report: &BotReport) -> Option<String> {
    if report.game_mode == Some(run.expected_mode) {
        return None;
    }
    Some(format!(
        "{side}: login game_mode {:?} — expected {} ({})",
        report.game_mode, run.expected_mode, run.gamemode
    ))
}

/// Waits until bot A has entered play; `false` when A's task ended before
/// that or the join budget expired.
async fn wait_for_a(shared: &Shared, a: &tokio::task::JoinHandle<BotReport>) -> bool {
    loop {
        if shared.a_joined.load(Ordering::SeqCst) {
            return true;
        }
        if a.is_finished() {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Runs one scenario: fresh v3 copy → template hash → A joins → B joins →
/// B stands 10 s and leaves → A observes 5 s → `save-all flush` + `stop` →
/// hash → report file. Never panics: failures are violations like any other
/// stop condition (the final assert decides).
async fn one_run(
    env: &Arc<ParityEnv>,
    ids: &Arc<PacketIds>,
    template: &Path,
    run: &'static ModeRun,
) -> RunOutcome {
    let mut outcome = RunOutcome::new(run);
    let template_before = hash_tree(template);

    let vanilla = match Vanilla::start_from(env, template).await {
        Ok(server) => server,
        Err(error) => {
            outcome.violations.push(format!("vanilla start: {error}"));
            write_report(&outcome);
            return outcome;
        }
    };
    let shared = Arc::new(Shared::new());
    let b_uuid = protocol::offline_uuid(BOT_B);

    let a = tokio::spawn(run_bot(
        Arc::clone(env),
        Arc::clone(ids),
        BOT_A,
        outcome.a_trace.clone(),
        SAFETY,
        Observer::new(Arc::clone(&shared), b_uuid),
    ));
    // B starts only after A is in play, so A's entity wire for B starts at a
    // known moment (join order is a scenario fact).
    let joined = tokio::time::timeout(JOIN_WAIT, wait_for_a(&shared, &a))
        .await
        .unwrap_or(false);
    let b = if joined {
        Some(tokio::spawn(run_bot(
            Arc::clone(env),
            Arc::clone(ids),
            BOT_B,
            outcome.b_trace.clone(),
            SAFETY,
            BScript::new(Scene::Join, Arc::clone(&shared)),
        )))
    } else {
        None
    };
    let ra = match a.await {
        Ok(report) => Ok(report),
        Err(error) => Err(format!("A task panicked: {error}")),
    };
    let rb = match b {
        Some(handle) => match handle.await {
            Ok(report) => Some(Ok(report)),
            Err(error) => Some(Err(format!("B task panicked: {error}"))),
        },
        None => None,
    };
    if let Err(error) = vanilla.stop_with_flush().await {
        outcome.violations.push(format!("server stop: {error}"));
    }

    if !joined {
        outcome
            .facts
            .push("bot A never entered play — bot B was not started".to_owned());
    }
    match ra {
        Ok(report) => {
            match mode_fact("A", run, &report) {
                Some(fact) => outcome.facts.push(fact),
                None => {
                    if let Some(problem) = mode_problem("A", run, &report) {
                        outcome.violations.push(problem);
                    }
                }
            }
            outcome.a = Some(report);
        }
        Err(error) => outcome.violations.push(error),
    }
    match rb {
        Some(Ok(report)) => {
            match mode_fact("B", run, &report) {
                Some(fact) => outcome.facts.push(fact),
                None => {
                    if let Some(problem) = mode_problem("B", run, &report) {
                        outcome.violations.push(problem);
                    }
                }
            }
            outcome.b = Some(report);
        }
        Some(Err(error)) => outcome.violations.push(error),
        None => {}
    }

    // The frozen template must stay byte-identical across the run.
    let after = hash_tree(template);
    outcome.template_changed = changed(&template_before, &after);
    if !outcome.template_changed.is_empty() {
        outcome.violations.push(format!(
            "the frozen v3 template changed during {}: {}",
            outcome.run.code,
            outcome.template_changed.join(", ")
        ));
    }

    write_report(&outcome);
    outcome
}

/// Bot A/B's report block (its own section of the run report).
fn bot_block(side: &str, bot: &str, run: &ModeRun, report: Option<&BotReport>) -> String {
    match report {
        None => format!("## Bot {side} ({bot})\nno report (task failed before play)\n"),
        Some(r) => format!(
            "## Bot {side} ({bot})\nend: {}\nserver player_position: {} | movement \
             packets sent: {} | {:.0} ms elapsed\nspawn: {:?} | last position: {:?} | \
             login game_mode: {:?} (expected {}, {})\n",
            r.end.text(),
            r.teleports,
            r.moves,
            r.elapsed_ms,
            r.spawn,
            r.last,
            r.game_mode,
            run.expected_mode,
            run.gamemode
        ),
    }
}

/// Writes one run's report file (facts only; conclusions go to the docs).
fn write_report(outcome: &RunOutcome) {
    let run = outcome.run;
    let mut text = format!(
        "# Two-bot M4.5 game-mode trace — {} ({})\n",
        run.desc, run.code
    );
    text.push_str(&format!(
        "server.properties: gamemode={} | force-gamemode=true | view-distance=10 | \
         simulation-distance=10\n",
        run.gamemode
    ));
    text.push_str(&bot_block("A", BOT_A, run, outcome.a.as_ref()));
    text.push('\n');
    text.push_str(&bot_block("B", BOT_B, run, outcome.b.as_ref()));
    if outcome.b.is_none() && outcome.a.is_some() {
        text.push_str("\nfact: bot B was not started (the join of bot A did not complete)\n");
    }
    text.push_str("\n## Facts\n");
    if outcome.facts.is_empty() {
        text.push_str("none\n");
    }
    for fact in &outcome.facts {
        text.push_str(fact);
        text.push('\n');
    }
    text.push_str("\n## v3 template SHA-256 diff (must stay empty)\n");
    if outcome.template_changed.is_empty() {
        text.push_str("none\n");
    } else {
        for line in &outcome.template_changed {
            text.push_str(line);
            text.push('\n');
        }
    }
    text.push_str("\n## Stop conditions found\n");
    if outcome.violations.is_empty() {
        text.push_str("none\n");
    }
    for violation in &outcome.violations {
        text.push_str(violation);
        text.push('\n');
    }
    text.push_str("\n## Traces\n");
    text.push_str(&format!("A JSONL: {}\n", outcome.a_trace.display()));
    if outcome.b.is_some() {
        text.push_str(&format!("B JSONL: {}\n", outcome.b_trace.display()));
    } else {
        text.push_str(&format!(
            "B JSONL: {} (B not started => absent)\n",
            outcome.b_trace.display()
        ));
    }
    if let Err(error) = std::fs::write(&outcome.report, text) {
        eprintln!("cannot write {}: {error}", outcome.report.display());
    }
}

/// C5 then C6: one run each on a fresh v3 copy with the forced game mode.
#[tokio::test]
#[ignore = "requires the owner's vanilla installation (C:\\MineRust-data)"]
async fn two_bot_gamemode_c5_c6() {
    let env = match load_parity_env() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            return;
        }
    };
    let ids = match env.packet_ids() {
        Ok(ids) => Arc::new(ids),
        Err(error) => {
            eprintln!("SKIP: packet ids unavailable: {error}");
            return;
        }
    };
    if let Err(error) = std::fs::create_dir_all(TRACE_DIR) {
        eprintln!("SKIP: cannot create {TRACE_DIR}: {error}");
        return;
    }
    let template = Path::new(V3_TEMPLATE);
    if !template.is_dir() {
        eprintln!("SKIP: v3 template not found at {V3_TEMPLATE}");
        return;
    }
    let env = Arc::new(env);

    // The properties guard restores the copy's `server.properties` whatever
    // happens below (the previous runs were recorded with gamemode=spectator).
    let original = match std::fs::read_to_string(env.vanilla_dir.join("server.properties")) {
        Ok(text) => text,
        Err(error) => {
            eprintln!("SKIP: cannot read server.properties: {error}");
            return;
        }
    };
    let _guard = PropertiesGuard {
        path: env.vanilla_dir.join("server.properties"),
        original,
    };

    let template_all_before = hash_tree(template);
    let mut performed: Vec<RunOutcome> = Vec::new();
    for run in &RUNS {
        if let Err(error) = set_properties(&env, run.gamemode) {
            eprintln!(
                "SKIP: cannot set server.properties for {}: {error}",
                run.code
            );
            return;
        }
        let outcome = one_run(&env, &ids, template, run).await;
        eprintln!(
            "{}: violations={:?} facts={}",
            outcome.run.code,
            outcome.violations,
            outcome.facts.len()
        );
        performed.push(outcome);
    }

    // The frozen template must be byte-identical across both runs.
    let template_all_after = hash_tree(template);
    let template_changed = changed(&template_all_before, &template_all_after);

    let summary_path = Path::new(TRACE_DIR).join("two-gamemode-summary.md");
    let mut summary = String::from("# M4.5 two-bot game-mode trace — summary\n\n");
    for outcome in &performed {
        summary.push_str(&format!(
            "- {} ({}): A = {} | B = {} | stop-conditions = {:?} | report = {}\n",
            outcome.run.code,
            outcome.run.desc,
            outcome
                .a
                .as_ref()
                .map(|r| r.end.text())
                .unwrap_or_else(|| "no report".to_owned()),
            outcome
                .b
                .as_ref()
                .map(|r| r.end.text())
                .unwrap_or_else(|| "not started".to_owned()),
            outcome.violations,
            outcome.report.display()
        ));
    }
    summary.push_str(&format!(
        "\ntemplate changed over both runs ({} files): {}\n",
        template_changed.len(),
        if template_changed.is_empty() {
            "none".to_owned()
        } else {
            template_changed.join(", ")
        }
    ));
    if let Err(error) = std::fs::write(&summary_path, summary) {
        eprintln!("cannot write {}: {error}", summary_path.display());
    }
    eprintln!("summary: {}", summary_path.display());

    // Report everything; panic only on the hard invariants: the frozen
    // template and the owner's stop conditions.
    assert!(
        template_changed.is_empty(),
        "STOP: the frozen v3 template changed — see two-gamemode-summary.md"
    );
    let violations: Vec<String> = performed
        .iter()
        .flat_map(|outcome| outcome.violations.iter().cloned())
        .collect();
    assert!(
        violations.is_empty(),
        "STOP: stop conditions found, repair first and rerun: {}\nsee two-gamemode-summary.md",
        violations.join(" | ")
    );
}
