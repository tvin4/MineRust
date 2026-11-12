//! M4.5 data step: the vanilla two-bot trace (С1/С2/С3).
//!
//! Three scenarios, one run each on a fresh copy of the frozen v3
//! template (`Vanilla::start` recopies it every time), bot A joined first
//! and staying at spawn for the whole scenario, bot B joining second under
//! its own name/offline UUID and following the scenario script:
//!
//! * С1 — B joins near A, stands 10 s, leaves cleanly (its connection is
//!   closed after the scripted end); A's JSONL records what vanilla showed.
//! * С2 — B walks +x 10 s (0.2 blocks per tick, one movement packet after
//!   each `client_tick_end`), alternating `move_player_pos`,
//!   `move_player_pos_rot` and `move_player_rot` with different yaw/pitch,
//!   then stands 5 s.
//! * С3 — B walks +x until vanilla stops showing B to A (a
//!   `remove_entities` carrying B's id while B did not leave), then walks
//!   back to the join point; the distance is the tracking estimate. Being
//!   refused before the ready-zone edge (assert before send) yields the
//!   fact «tracking distance: не установлено», never a silent guess.
//!
//! Stop conditions (owner's list): kicks/closed connections, rollback (a
//! second server `player_position`), movement refused outside the ready
//! zone (violation in С1/С2, the «не установлено» fact in С3), failures
//! outside bot code. New chunks vanilla creates outside the v3 ready zone
//! are *listed* by the world criterion, never a silent stop; a criterion
//! that cannot run is a violation (unproven evidence).
//!
//! Bot-to-bot coordination is atomic flags + shared live facts (see
//! `common/twobot.rs`); there is no second `Mutex` in the hot path — the
//! server is vanilla and untouched: bot code only, no server-crate change
//! is required or made by this step.
//!
//! Outputs: `<code>-A.jsonl` / `<code>-B.jsonl` (A's clock is its own
//! connection's monotonic time; B's clock is B's — cross-references use
//! the shared live flags, not `t_ms`), `<code>.report.txt` and
//! `twobot-summary.md` under `C:\MineRust-data\traces\`.

#[path = "common/twobot_scenarios.rs"]
mod scenarios;
#[path = "common/twobot.rs"]
mod twobot;
#[path = "common/world_criterion.rs"]
mod world_criterion;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use parity::{load_parity_env, ParityEnv, Vanilla};
use scenarios::{BScript, Observer, Scene, EXPECTED_GAME_MODE};
use sha2::{Digest, Sha256};
use twobot::{run_bot, BotReport, End, Shared, BOT_A, BOT_B};

/// Where traces and reports land (outside the repository, by design).
const TRACE_DIR: &str = r"C:\MineRust-data\traces";
/// Budget for both bots to reach play (each scripted step has its own
/// waits on top; this only caps a hang that never enters play).
const JOIN_WAIT: Duration = Duration::from_secs(180);

/// Everything one scenario produced, for its report and the summary.
struct ScenarioOutcome {
    /// File-name code (`c1`/`c2`/`c3`).
    code: &'static str,
    /// Human description of the scenario.
    desc: &'static str,
    /// Bot A's JSONL path.
    a_trace: PathBuf,
    /// Bot B's JSONL path.
    b_trace: PathBuf,
    /// The per-scenario report path.
    report: PathBuf,
    /// Bot A's final report (absent only if its task panicked).
    a: Option<BotReport>,
    /// Bot B's final report (`None` when B was never started).
    b: Option<BotReport>,
    /// What A observed live on the entity wire (ids, counts, strategy).
    a_observed: String,
    /// Report facts (script ends, world lists, distances).
    facts: Vec<String>,
    /// The owner's stop conditions found by this scenario.
    violations: Vec<String>,
    /// Parsed `chunk_status.py compare <template> <copy>` facts.
    world: Option<world_criterion::WorldCriterion>,
    /// Files of the frozen template that changed during the scenario.
    template_changed: Vec<String>,
}

impl ScenarioOutcome {
    /// Fresh outcome with the paths of one scenario.
    fn new(scene: Scene) -> Self {
        let code = scene.code();
        let dir = Path::new(TRACE_DIR);
        Self {
            code,
            desc: scene_desc(scene),
            a_trace: dir.join(format!("two-{code}-A.jsonl")),
            b_trace: dir.join(format!("two-{code}-B.jsonl")),
            report: dir.join(format!("two-{code}.report.txt")),
            a: None,
            b: None,
            a_observed: "bot A never produced a report".to_owned(),
            facts: Vec::new(),
            violations: Vec::new(),
            world: None,
            template_changed: Vec::new(),
        }
    }
}

/// One-line description of a scenario (report header and summary).
fn scene_desc(scene: Scene) -> &'static str {
    match scene {
        Scene::Join => "С1: B joins, stands 10 s, leaves cleanly",
        Scene::Walk => "С2: B walks +x 10 s with rotation, stands 5 s",
        Scene::Track => "С3: B walks until A loses track of B, then returns",
    }
}

/// Per-bot play-loop budget: joins plus the script plus a margin.
fn safety_for(scene: Scene) -> Duration {
    match scene {
        Scene::Join => Duration::from_secs(180),
        Scene::Walk => Duration::from_secs(240),
        Scene::Track => Duration::from_secs(480),
    }
}

/// The v3 ready zone in chunk coordinates (the same numbers as
/// `twobot::in_ready_zone`, checked here against already-chunked ids).
fn in_v3_chunk(at: (i32, i32)) -> bool {
    (-14..=26).contains(&at.0) && (-22..=18).contains(&at.1)
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

/// The world-criterion diagnostic script next to this test file, under
/// the workspace root.
fn chunk_status_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("parity")
        .join("chunk_status.py")
}

/// `"none"` or one `(x, z)` per chunk, for the report's lists.
fn fmt_coords(items: &[world_criterion::Coord]) -> String {
    if items.is_empty() {
        return "none".to_owned();
    }
    items
        .iter()
        .map(|(x, z)| format!("({x}, {z})"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Classifies one bot's end against the owner's stop conditions:
/// a scripted end is a fact, the С3 bounds end is the «не установлено»
/// fact, every other end is a violation (kick, rollback, timeout, io).
fn judge(
    scene: Scene,
    side: &str,
    end: &End,
    facts: &mut Vec<String>,
    violations: &mut Vec<String>,
) {
    match end {
        End::Script(text) => facts.push(format!("{side} finished: {text}")),
        End::Bounds(text) if scene == Scene::Track => {
            facts.push(format!("{side}: {text}"));
        }
        other => violations.push(format!("{side}: {}", other.text())),
    }
}

/// Spectator must be proven from every bot's own trace (login
/// `game_mode`); anything else is a stop condition like any other.
fn mode_problem(side: &str, report: &BotReport) -> Option<String> {
    if report.game_mode == Some(EXPECTED_GAME_MODE) {
        return None;
    }
    Some(format!(
        "{side}: login game_mode {:?} — expected {EXPECTED_GAME_MODE} (spectator)",
        report.game_mode
    ))
}

/// Waits until bot A has entered play; `false` when A's task ended
/// before that (its run returns instantly on connect/join failure) or
/// the scenario's join budget expired.
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

/// The entity-detection line A's JSONL proves afterwards: counts, the
/// id it attributed to B, and which strategy produced it.
fn observed_facts(shared: &Shared) -> String {
    let adds = shared.adds.load(Ordering::SeqCst);
    let removes = shared.removes.load(Ordering::SeqCst);
    let entity = shared.b_entity.load(Ordering::SeqCst);
    let index = shared.entity_add_index.load(Ordering::SeqCst);
    let strategy = if entity < 0 {
        "не установлено (neither strategy matched)".to_owned()
    } else if shared.uuid_strategy.load(Ordering::SeqCst) {
        format!("offline UUID match on add #{}", index + 1)
    } else {
        format!(
            "fallback: first add_entity after B joined (add #{})",
            index + 1
        )
    };
    format!(
        "A saw add_entity ×{adds}, remove_entities ×{removes} (id lists parsed); \
         B entity id = {entity}, strategy: {strategy}"
    )
}

/// Runs one scenario: fresh copy → template hash → A joins → B joins →
/// both scripts end → `save-all flush` + `stop` → hash → world criterion
/// → report file. Never panics: a dead server is a violation like any
/// other stop condition (the final assert decides).
async fn one_scenario(env: &ParityEnv, ids: Arc<PacketIds>, scene: Scene) -> ScenarioOutcome {
    let mut outcome = ScenarioOutcome::new(scene);
    let template_before = hash_tree(&env.world_template);

    let vanilla = match Vanilla::start(env).await {
        Ok(server) => server,
        Err(error) => {
            outcome.violations.push(format!("vanilla start: {error}"));
            write_report(&outcome);
            return outcome;
        }
    };
    let world_dir = env.vanilla_dir.join("world");
    let env = Arc::new(env.clone());
    let shared = Arc::new(Shared::new());
    let quit = Arc::new(AtomicBool::new(false));
    let safety = safety_for(scene);
    let b_uuid = protocol::offline_uuid(BOT_B);

    // The bots play while the server future only waits for the `quit`
    // flag and then performs `save-all flush` + `stop` (the walk order:
    // the `stop` is issued strictly after every bot returned).
    let run = {
        let env_a = Arc::clone(&env);
        let env_b = Arc::clone(&env);
        let ids_a = Arc::clone(&ids);
        let ids_b = Arc::clone(&ids);
        let shared_a = Arc::clone(&shared);
        let shared_b = Arc::clone(&shared);
        let shared_wait = Arc::clone(&shared);
        let path_a = outcome.a_trace.clone();
        let path_b = outcome.b_trace.clone();
        async move {
            let a = tokio::spawn(run_bot(
                env_a,
                ids_a,
                BOT_A,
                path_a,
                safety,
                Observer::new(shared_a, b_uuid),
            ));
            // B starts only after A is in play, so A's entity wire for B
            // starts at a known moment (join order is a scenario fact).
            let joined = tokio::time::timeout(JOIN_WAIT, wait_for_a(&shared_wait, &a))
                .await
                .unwrap_or(false);
            let b = if joined {
                Some(tokio::spawn(run_bot(
                    env_b,
                    ids_b,
                    BOT_B,
                    path_b,
                    safety,
                    BScript::new(scene, shared_b),
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
            (ra, rb, joined)
        }
    };
    let quit_run = Arc::clone(&quit);
    let runner = async move {
        let result = run.await;
        quit_run.store(true, Ordering::SeqCst);
        result
    };
    let stopper = async {
        while !quit.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        vanilla.stop_with_flush().await
    };
    let ((ra, rb, joined), stop_res) = tokio::join!(runner, stopper);

    if !joined {
        outcome
            .facts
            .push("bot A never entered play — bot B was not started".to_owned());
    }
    match ra {
        Ok(report) => {
            judge(
                scene,
                "A",
                &report.end,
                &mut outcome.facts,
                &mut outcome.violations,
            );
            if let Some(problem) = mode_problem("A", &report) {
                outcome.violations.push(problem);
            }
            outcome.a = Some(report);
        }
        Err(error) => outcome.violations.push(error),
    }
    match rb {
        Some(Ok(report)) => {
            judge(
                scene,
                "B",
                &report.end,
                &mut outcome.facts,
                &mut outcome.violations,
            );
            if let Some(problem) = mode_problem("B", &report) {
                outcome.violations.push(problem);
            }
            outcome.b = Some(report);
        }
        Some(Err(error)) => outcome.violations.push(error),
        None => {}
    }
    if let Err(error) = stop_res {
        outcome.violations.push(format!("server stop: {error}"));
    }
    outcome.facts.push(observed_facts(&shared));

    // The frozen template must stay byte-identical across the scenario.
    let after = hash_tree(&env.world_template);
    outcome.template_changed = changed(&template_before, &after);
    if !outcome.template_changed.is_empty() {
        outcome.violations.push(format!(
            "the frozen v3 template changed during {}: {}",
            outcome.code,
            outcome.template_changed.join(", ")
        ));
    }

    // The world criterion: new chunks are split against the v3 ready
    // zone and listed — per the owner's rule a zone exit is listed, not
    // a stop; a criterion that could not run stops as unproven.
    let case = world_criterion::compare(&chunk_status_script(), &env.world_template, &world_dir);
    match &case.error {
        Some(error) => outcome.violations.push(format!("world criterion: {error}")),
        None => {
            let (inside, outside): (Vec<_>, Vec<_>) = case
                .new_chunks
                .iter()
                .copied()
                .partition(|at| in_v3_chunk(*at));
            let zone_text = "x -14..26 / z -22..18";
            outcome.facts.push(format!(
                "world: new chunks inside the v3 ready zone {zone_text} ({}): {}",
                inside.len(),
                fmt_coords(&inside)
            ));
            outcome.facts.push(format!(
                "world: new chunks OUTSIDE the v3 ready zone (listed, no stop) ({}): {}",
                outside.len(),
                fmt_coords(&outside)
            ));
            outcome.facts.push(format!(
                "world: status_diffs {} inside / {} outside the zone; template {} / copy {} \
                 chunks; missing {} (all listed, no stop)",
                case.status_inside_zone().len(),
                case.status_outside_zone().len(),
                case.template_chunks,
                case.copy_chunks,
                case.missing_chunks.len()
            ));
        }
    }
    outcome.world = Some(case);

    write_report(&outcome);
    outcome
}

/// Bot A's report block (its own section of the scenario report).
fn bot_block(side: &str, bot: &str, report: Option<&BotReport>) -> String {
    match report {
        None => format!("## Bot {side} ({bot})\nno report (task failed before play)\n"),
        Some(r) => format!(
            "## Bot {side} ({bot})\nend: {}\nserver player_position: {} | movement \
             packets sent: {} | {:.0} ms elapsed\nspawn: {:?} | last position: {:?} | \
             login game_mode: {:?} (expected {EXPECTED_GAME_MODE})\n",
            r.end.text(),
            r.teleports,
            r.moves,
            r.elapsed_ms,
            r.spawn,
            r.last,
            r.game_mode
        ),
    }
}

/// Writes one scenario's report file (facts only; conclusions go to the
/// docs; coordinate lists stay here and out of the session report).
fn write_report(outcome: &ScenarioOutcome) {
    let mut text = format!(
        "# Two-bot M4.5 trace — {} ({})\n",
        outcome.desc, outcome.code
    );
    text.push_str(&bot_block("A", BOT_A, outcome.a.as_ref()));
    text.push('\n');
    text.push_str(&bot_block("B", BOT_B, outcome.b.as_ref()));
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
    text.push_str("\n## World criterion (chunk_status.py compare <template> <copy>)\n");
    match &outcome.world {
        None => text.push_str("not run (vanilla never started)\n"),
        Some(case) => match &case.error {
            Some(error) => text.push_str(&format!("error: {error}\n")),
            None => {
                let (inside, outside): (Vec<_>, Vec<_>) = case
                    .new_chunks
                    .iter()
                    .copied()
                    .partition(|at| in_v3_chunk(*at));
                text.push_str("zone (v3 ready): x -14..26 | z -22..18\n");
                text.push_str(&format!(
                    "new chunks inside the zone: {}\n",
                    fmt_coords(&inside)
                ));
                text.push_str(&format!(
                    "new chunks outside the zone (list, no stop): {}\n",
                    fmt_coords(&outside)
                ));
                text.push_str(&format!(
                    "missing from the copy (info): {}\n",
                    fmt_coords(&case.missing_chunks)
                ));
            }
        },
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
    text.push_str(&format!(
        "B JSONL: {} (B not started => absent)\n",
        outcome.b_trace.display()
    ));

    if let Err(error) = std::fs::write(&outcome.report, text) {
        eprintln!("cannot write {}: {error}", outcome.report.display());
    }
}

/// The three scenarios of M4.5; a failing scenario stops the sequence
/// (fix, then rerun — one run each otherwise, by design).
#[tokio::test]
#[ignore = "requires the owner's vanilla installation (C:\\MineRust-data)"]
async fn two_bot_trace_three_scenarios() {
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
    let template_before = hash_tree(&env.world_template);

    let mut performed: Vec<ScenarioOutcome> = Vec::new();
    let mut stopped_after: Option<&str> = None;
    for scene in [Scene::Join, Scene::Walk, Scene::Track] {
        let outcome = one_scenario(&env, Arc::clone(&ids), scene).await;
        let failed = !outcome.violations.is_empty();
        eprintln!(
            "{}: violations={:?} facts={}",
            outcome.code,
            outcome.violations,
            outcome.facts.len()
        );
        performed.push(outcome);
        if failed {
            stopped_after = Some(scene.code());
            break;
        }
    }

    // The frozen template must be byte-identical across all scenarios.
    let template_after = hash_tree(&env.world_template);
    let template_changed = changed(&template_before, &template_after);

    let summary_path = Path::new(TRACE_DIR).join("twobot-summary.md");
    let mut summary = String::from("# M4.5 two-bot vanilla trace — summary\n\n");
    for outcome in &performed {
        summary.push_str(&format!(
            "- {} ({}): A = {} | B = {} | stop-conditions = {:?} | report = {}\n",
            outcome.code,
            outcome.desc,
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
        summary.push_str(&format!("  - {}\n", outcome.a_observed));
    }
    summary.push_str(&format!(
        "\ntemplate changed over all scenarios ({} files): {}\n",
        template_changed.len(),
        if template_changed.is_empty() {
            "none".to_owned()
        } else {
            template_changed.join(", ")
        }
    ));
    summary.push_str(&format!(
        "stopped after: {}\n",
        stopped_after.unwrap_or("nothing — all three scenarios completed")
    ));
    if let Err(error) = std::fs::write(&summary_path, summary) {
        eprintln!("cannot write {}: {error}", summary_path.display());
    }
    eprintln!("summary: {}", summary_path.display());

    // Report everything; panic only on the hard invariants: the frozen
    // template and the owner's stop conditions.
    assert!(
        template_changed.is_empty(),
        "STOP: the frozen v3 template changed — see twobot-summary.md"
    );
    let violations: Vec<String> = performed
        .iter()
        .flat_map(|outcome| outcome.violations.iter().cloned())
        .collect();
    assert!(
        violations.is_empty(),
        "STOP: stop conditions found, repair first and rerun (one run each): {}\n\
         see twobot-summary.md",
        violations.join(" | ")
    );
}
