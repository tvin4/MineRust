//! ЧАСТЬ I diagnostic: what triggers the out-of-zone chunk strip?
//!
//! One vanilla run per fresh v2 copy, selected by `MINECRAFT_IDLE`:
//! * `nobot` — start vanilla, wait 30 s, `save-all flush` + stop;
//! * `stand` — a bot joins and stands 30 s (no movement);
//! * `walk`  — the bot walks straight +x like П2 (302 steps) and settles;
//! * `walk50`— the same walk but only 50 steps (~10 blocks, less than a chunk).
//!
//! For `walk*` the world's region files are polled every 2 s so the moment
//! generation overflows into `r.1.*` (chunk x ≥ 32) is visible. Reports land
//! in `C:\MineRust-data\traces\idle-<case>.report.txt`.

#[path = "common/world_criterion.rs"]
mod world_criterion;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parity::{load_parity_env, run_walk, ParityEnv, RunStats, Vanilla, WalkMode};

const TRACE_DIR: &str = r"C:\MineRust-data\traces";

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

fn category(at: (i32, i32)) -> &'static str {
    match (at.0 > 29, at.1 > 21) {
        (true, true) => "east+north corner",
        (true, false) => "east (+x strip)",
        (false, true) => "north (+z strip)",
        (false, false) => "unclassified",
    }
}

fn summarize(case: &world_criterion::WorldCriterion) -> String {
    let outside = case.new_outside_zone();
    let mut by_cat: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for at in &outside {
        *by_cat.entry(category(*at)).or_default() += 1;
    }
    format!(
        "template {} | copy {} | new {} | outside {} {:?} | missing {}",
        case.template_chunks,
        case.copy_chunks,
        case.new_chunks.len(),
        outside.len(),
        by_cat,
        case.missing_chunks.len()
    )
}

/// Region files under `world` with their sizes, plus whether `r.1.*` exists.
fn region_snapshot(world: &Path) -> String {
    let region = world.join("dimensions/minecraft/overworld/region");
    let mut files: Vec<(String, u64)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&region) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                files.push((entry.file_name().to_string_lossy().into_owned(), meta.len()));
            }
        }
    }
    files.sort();
    let total: u64 = files.iter().map(|(_, n)| n).sum();
    let r1 = files.iter().any(|(n, _)| n.starts_with("r.1."));
    format!("total {total} B | r.1 present: {r1} | {files:?}")
}

fn walk_mode(steps: u32) -> WalkMode {
    WalkMode::WalkLoaded {
        diagonal: false,
        steps,
        settle: Duration::from_secs(2),
        on_ground: true,
        alternate_rot: false,
        tick_coupled: true,
    }
}

async fn run_one(env: &ParityEnv, ids: Arc<data::PacketIds>, which: &str) -> String {
    let report = Path::new(TRACE_DIR).join(format!("idle-{which}.report.txt"));
    let world = env.vanilla_dir.join("world");
    let mut text = format!("# idle-{which}\n");

    let server = match Vanilla::start(env).await {
        Ok(server) => server,
        Err(error) => return format!("{text}vanilla start failed: {error}\n"),
    };
    text.push_str(&format!("start: {}\n", region_snapshot(&world)));

    let stats: Option<RunStats> = match which {
        "nobot" => {
            for i in 1..=15 {
                tokio::time::sleep(Duration::from_secs(2)).await;
                if i % 5 == 0 {
                    text.push_str(&format!("t={}s {}\n", i * 2, region_snapshot(&world)));
                }
            }
            None
        }
        "stand" => {
            let trace = Path::new(TRACE_DIR).join("idle-stand.jsonl");
            let walk = run_walk(
                env,
                ids,
                WalkMode::Stand {
                    hold: Duration::from_secs(30),
                },
                &trace,
            );
            let poll = async {
                for i in 1..=15 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    eprintln!("t={}s {}", i * 2, region_snapshot(&world));
                }
            };
            let (walk, ()) = tokio::join!(walk, poll);
            match walk {
                Ok(stats) => Some(stats),
                Err(error) => {
                    text.push_str(&format!("walk failed: {error}\n"));
                    None
                }
            }
        }
        other => {
            let steps = if other == "walk50" { 50 } else { 302 };
            let trace = Path::new(TRACE_DIR).join(format!("idle-{other}.jsonl"));
            let walk = run_walk(env, ids, walk_mode(steps), &trace);
            let poll = async {
                for i in 1..=20 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    eprintln!("t={}s {}", i * 2, region_snapshot(&world));
                }
            };
            let (walk, ()) = tokio::join!(walk, poll);
            match walk {
                Ok(stats) => Some(stats),
                Err(error) => {
                    text.push_str(&format!("walk failed: {error}\n"));
                    None
                }
            }
        }
    };

    text.push_str(&format!("pre-stop: {}\n", region_snapshot(&world)));
    if let Some(stats) = &stats {
        text.push_str(&format!(
            "stop: {:?} | elapsed {:.0} ms | moves {}\n",
            stats.stop, stats.elapsed_ms, stats.moves
        ));
    }
    if let Err(error) = server.stop_with_flush().await {
        text.push_str(&format!("server stop: {error}\n"));
    }
    let case = world_criterion::compare(&chunk_status_script(), &env.world_template, &world);
    text.push_str(&summarize(&case));
    text.push('\n');
    if let Err(error) = std::fs::write(&report, &text) {
        eprintln!("cannot write {}: {error}", report.display());
    }
    text
}

#[tokio::test]
#[ignore = "requires the owner's vanilla installation (C:\\MineRust-data)"]
async fn idle_strip_cause() {
    let env = match parity_env_or_skip() {
        Some(env) => env,
        None => return,
    };
    let ids = Arc::new(env.packet_ids().expect("packet ids"));
    let which = std::env::var("MINECRAFT_IDLE").unwrap_or_else(|_| "nobot".to_owned());
    eprintln!("idle-{which}: {}", run_one(&env, ids, &which).await);
}
