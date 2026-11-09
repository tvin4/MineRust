//! ЧАСТЬ I: the kick probe. One experiment per invocation on a fresh v2
//! copy (`Vanilla::start` copies the frozen template every time):
//!
//! * `MINECRAFT_PROBE=e1` (default) — Э1: the old П2 walk (straight +x,
//!   0.2 steps, 50 ms grid, flag 0x00) but the first move waits for
//!   `player_loaded`;
//! * `MINECRAFT_PROBE=e2` — Э2: after `player_loaded`, the live client's
//!   first three `move_player_pos` byte for byte at their offsets, then a
//!   static 5 s idle;
//! * `MINECRAFT_PROBE=e3` — Э3: after `player_loaded`, alternate
//!   `move_player_pos` / `move_player_pos_rot` with the live flags (0x01).
//!
//! If the experiment completes without a stop condition, the test then
//! runs the owner's П2 (straight to x ≤ 157, ~302 steps) and П3 (diagonal
//! +x/+z to the 2-block reserve, ~427 steps) with the winning payload
//! style, each on its own fresh copy.
//!
//! Reports land in `C:\MineRust-data\traces\kick-probe-*.md`; the
//! repository keeps only this source.

#[path = "common/world_criterion.rs"]
mod world_criterion;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use parity::{load_parity_env, run_walk, ParityEnv, RunStats, Stop, Vanilla, WalkMode};

const TRACE_DIR: &str = r"C:\MineRust-data\traces";
const SETTLE: Duration = Duration::from_secs(5);
/// Old П2 length (the experiment keeps it).
const EXP_STEPS: u32 = 200;
/// New П2: straight +x up to x ≤ 157 (302 × 0.2 blocks).
const P2_STEPS: u32 = 302;
/// New П3: diagonal +x/+z up to the 2-block reserve (427 × 0.2/√2).
const P3_STEPS: u32 = 427;
const EXPECTED_GAME_MODE: i32 = 3;
const EXPECTED_ABILITIES: u8 = 0x07;

fn chunk_status_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("parity")
        .join("chunk_status.py")
}

fn parity_env_or_skip() -> Option<ParityEnv> {
    match load_parity_env() {
        Ok(env) => Some(env),
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            None
        }
    }
}

struct Outcome {
    name: String,
    label: String,
    stats: Option<RunStats>,
    fatal: Option<String>,
    violations: Vec<String>,
    world: Option<world_criterion::WorldCriterion>,
}

fn fmt_coords(items: &[world_criterion::Coord]) -> String {
    if items.is_empty() {
        "none".to_owned()
    } else {
        items
            .iter()
            .map(|(x, z)| format!("({x}, {z})"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn fmt_status(items: &[&world_criterion::StatusDiff]) -> String {
    if items.is_empty() {
        "none".to_owned()
    } else {
        items
            .iter()
            .map(|row| format!("({}, {}) {}->{}", row.at.0, row.at.1, row.from, row.to))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Runs one mode on a fresh copy and judges the owner's stop conditions.
async fn one(
    env: &ParityEnv,
    ids: Arc<PacketIds>,
    name: &str,
    label: &str,
    mode: WalkMode,
) -> Outcome {
    let trace = Path::new(TRACE_DIR).join(format!("{name}.jsonl"));
    let report = Path::new(TRACE_DIR).join(format!("{name}.report.txt"));
    let mut out = Outcome {
        name: name.to_owned(),
        label: label.to_owned(),
        stats: None,
        fatal: None,
        violations: Vec::new(),
        world: None,
    };
    let server = match Vanilla::start(env).await {
        Ok(server) => server,
        Err(error) => {
            out.fatal = Some(format!("vanilla start: {error}"));
            out.violations.push(out.fatal.clone().unwrap_or_default());
            return out;
        }
    };
    let walk = run_walk(env, Arc::clone(&ids), mode, &trace).await;
    let stop = server.stop_with_flush().await;
    let world = env.vanilla_dir.join("world");
    out.world = Some(world_criterion::compare(
        &chunk_status_script(),
        &env.world_template,
        &world,
    ));
    match walk {
        Ok(stats) => {
            if stats.stop != Stop::Completed {
                out.violations
                    .push(format!("stop reason: {:?}", stats.stop));
            }
            if stats.teleports > 1 {
                out.violations.push(format!(
                    "server player_position after the first: {} (rollback)",
                    stats.teleports
                ));
            }
            let mode_ok = stats.game_mode == Some(EXPECTED_GAME_MODE)
                && stats.abilities_flags == Some(EXPECTED_ABILITIES);
            if !mode_ok {
                out.violations.push(format!(
                    "mode check: game_mode {:?}, abilities {:?} (expected {EXPECTED_GAME_MODE}/0x{EXPECTED_ABILITIES:02X})",
                    stats.game_mode, stats.abilities_flags
                ));
            }
            out.stats = Some(stats);
        }
        Err(error) => {
            out.fatal = Some(format!("walk failed: {error}"));
            out.violations.push(out.fatal.clone().unwrap_or_default());
        }
    }
    if let Err(error) = stop {
        out.violations.push(format!("server stop: {error}"));
    }
    if let Some(case) = &out.world {
        if let Some(error) = &case.error {
            out.violations.push(format!("world criterion: {error}"));
        } else if !case.new_outside_zone().is_empty() {
            out.violations.push(format!(
                "world criterion: new chunks outside x {}..{} / z {}..{}: {}",
                world_criterion::ZONE_X.0,
                world_criterion::ZONE_X.1,
                world_criterion::ZONE_Z.0,
                world_criterion::ZONE_Z.1,
                fmt_coords(&case.new_outside_zone())
            ));
        }
    }

    // Per-run report.
    let mut text = String::new();
    text.push_str(&format!("# {name} — {label}\n"));
    text.push_str(&format!(
        "fatal: {}\n",
        out.fatal.as_deref().unwrap_or("none")
    ));
    if let Some(stats) = &out.stats {
        text.push_str(&format!(
            "stop: {:?}\nelapsed: {:.0} ms | server player_position: {} | moves: {}\n\
             spawn: {:?} | last: {:?}\nmode: login {:?} | abilities {:?}\n",
            stats.stop,
            stats.elapsed_ms,
            stats.teleports,
            stats.moves,
            stats.spawn,
            stats.last,
            stats.game_mode,
            stats.abilities_flags
        ));
        text.push_str("\n## Move-time counters (selected)\n");
        for (key, value) in &stats.counts {
            if key.contains("move_player")
                || key.contains("client_tick_end")
                || key.contains("player_loaded")
                || key.contains("accept_teleportation")
            {
                text.push_str(&format!("{key}: {value}\n"));
            }
        }
        text.push_str("\n## Batches\n");
        for batch in &stats.batches {
            text.push_str(&format!(
                "t={:.0} ms: {} chunks, {} bytes\n",
                batch.t_ms, batch.chunks, batch.bytes
            ));
        }
        text.push_str("\n## Window moves\n");
        for mv in &stats.cache_moves {
            text.push_str(&format!(
                "t={:.0} ms {} ({}, {})\n",
                mv.t_ms, mv.name, mv.x, mv.z
            ));
        }
    }
    if let Some(case) = &out.world {
        text.push_str(&format!(
            "\n## World criterion\nchunks: template {} | copy {} | new {} | missing {}\n\
             new inside zone: {}\nnew outside zone (STOP): {}\n\
             status inside: {}\nstatus outside: {}\nheightmap_diffs: {} | light_presence_diffs: {}\n",
            case.template_chunks,
            case.copy_chunks,
            case.new_chunks.len(),
            case.missing_chunks.len(),
            fmt_coords(&case.new_inside_zone()),
            fmt_coords(&case.new_outside_zone()),
            fmt_status(&case.status_inside_zone()),
            fmt_status(&case.status_outside_zone()),
            case.heightmap_diffs,
            case.light_diffs
        ));
    }
    text.push_str("\n## Stop conditions found\n");
    if out.violations.is_empty() {
        text.push_str("none\n");
    }
    for violation in &out.violations {
        text.push_str(violation);
        text.push('\n');
    }
    text.push_str(&format!("\nTrace: {}\n", trace.display()));
    if let Err(error) = std::fs::write(&report, text) {
        eprintln!("cannot write {}: {error}", report.display());
    }
    out
}

fn p2_mode(which: &str) -> WalkMode {
    let (on_ground, alternate_rot) = match which {
        "e2" | "e3" => (true, which == "e3"),
        _ => (false, false),
    };
    WalkMode::WalkLoaded {
        diagonal: false,
        steps: P2_STEPS,
        settle: SETTLE,
        on_ground,
        alternate_rot,
        tick_coupled: true,
    }
}

fn p3_mode(which: &str) -> WalkMode {
    let (on_ground, alternate_rot) = match which {
        "e2" | "e3" => (true, which == "e3"),
        _ => (false, false),
    };
    WalkMode::WalkLoaded {
        diagonal: true,
        steps: P3_STEPS,
        settle: SETTLE,
        on_ground,
        alternate_rot,
        tick_coupled: true,
    }
}

#[tokio::test]
#[ignore = "requires the owner's vanilla installation (C:\\MineRust-data)"]
async fn kick_probe_runs() {
    let env = match parity_env_or_skip() {
        Some(env) => env,
        None => return,
    };
    let ids = Arc::new(env.packet_ids().expect("packet ids"));
    if let Err(error) = std::fs::create_dir_all(TRACE_DIR) {
        eprintln!("SKIP: cannot create {TRACE_DIR}: {error}");
        return;
    }
    let which = std::env::var("MINECRAFT_PROBE").unwrap_or_else(|_| "e1".to_owned());
    let (name, label, mode) = match which.as_str() {
        "e2" => (
            "probe-e2",
            "Э2: live first three moves after player_loaded, then 5 s idle",
            WalkMode::LiveFirstMoves { idle: SETTLE },
        ),
        "e3" => (
            "probe-e3",
            "Э3: alternate move_player_pos / move_player_pos_rot, flag 0x01",
            WalkMode::WalkLoaded {
                diagonal: false,
                steps: EXP_STEPS,
                settle: SETTLE,
                on_ground: true,
                alternate_rot: true,
                tick_coupled: false,
            },
        ),
        _ => (
            "probe-e1",
            "Э1: old П2 walk, first move after player_loaded",
            WalkMode::WalkLoaded {
                diagonal: false,
                steps: EXP_STEPS,
                settle: SETTLE,
                on_ground: false,
                alternate_rot: false,
                tick_coupled: false,
            },
        ),
    };

    let mut runs = Vec::new();
    let experiment = one(&env, Arc::clone(&ids), name, label, mode).await;
    eprintln!(
        "{name}: stop={:?} violations={:?}",
        experiment.stats.as_ref().map(|s| format!("{:?}", s.stop)),
        experiment.violations
    );
    let passed = experiment.violations.is_empty();
    runs.push(experiment);

    let mut summary = String::from("# ЧАСТЬ I kick probe\n\n");
    summary.push_str(&format!("experiment: `{which}`\n\n"));
    if passed {
        for (run_name, label, mode) in [
            (
                "probe-p2",
                "П2: straight +x to x <= 157 (302 steps)",
                p2_mode(&which),
            ),
            (
                "probe-p3",
                "П3: diagonal +x/+z to the 2-block reserve (427 steps)",
                p3_mode(&which),
            ),
        ] {
            let outcome = one(&env, Arc::clone(&ids), run_name, label, mode).await;
            eprintln!(
                "{run_name}: stop={:?} violations={:?}",
                outcome.stats.as_ref().map(|s| format!("{:?}", s.stop)),
                outcome.violations
            );
            runs.push(outcome);
        }
    }

    for run in &runs {
        summary.push_str(&format!("## {} — {}\n", run.name, run.label));
        if let Some(stats) = &run.stats {
            summary.push_str(&format!(
                "- stop: {:?}\n- elapsed: {:.0} ms | teleports: {} | moves: {}\n\
                 - spawn: {:?} | last: {:?}\n- mode: login {:?} | abilities {:?}\n",
                stats.stop,
                stats.elapsed_ms,
                stats.teleports,
                stats.moves,
                stats.spawn,
                stats.last,
                stats.game_mode,
                stats.abilities_flags
            ));
        }
        if let Some(case) = &run.world {
            summary.push_str(&format!(
                "- world: new {} in-zone / {} out-of-zone; status_diffs {} in / {} out; \
                 heightmap {} | light {}\n",
                case.new_inside_zone().len(),
                case.new_outside_zone().len(),
                case.status_inside_zone().len(),
                case.status_outside_zone().len(),
                case.heightmap_diffs,
                case.light_diffs
            ));
        }
        summary.push_str(&format!("- violations: {:?}\n\n", run.violations));
    }
    summary.push_str(&format!("experiment passed: {passed}\n"));
    let path = Path::new(TRACE_DIR).join(format!("kick-probe-{which}.md"));
    if let Err(error) = std::fs::write(&path, summary) {
        eprintln!("cannot write {}: {error}", path.display());
    }
    eprintln!("summary: {}", path.display());
}
