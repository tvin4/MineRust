//! World-window probe: one walk run per invocation, with the produced world
//! kept on disk so the owner's new world criterion can be read from `.mca`.
//!
//! The criterion replaced the old "no new chunks outside the zone": now
//! (a) chunks inside the send window (radius 10 around the current centre)
//! must not change status/content, and (b) new chunks outside the window are
//! expected vanilla look-ahead, listed but not a stop.
//!
//! `MINECRAFT_WINDOW_PROBE=p2` — v2, straight +x, 302 steps (ЧАСТЬ I П2)
//! `MINECRAFT_WINDOW_PROBE=p3` — v2, diagonal +x/+z, 427 steps (ЧАСТЬ I П3)
//! `MINECRAFT_WINDOW_PROBE=p4` — v3, straight +x, 600 steps (~120 blocks)
//! `MINECRAFT_WINDOW_PROBE=p5` — v3, diagonal +x/+z, 600 steps (~120 blocks)
//!
//! The run itself repeats the Part I winner exactly (`on_ground: true`,
//! `alternate_rot: false`, `tick_coupled: true`, 0.2 steps on the 50 ms
//! grid). The world of the run is copied to
//! `C:\MineRust-data\world-window-<name>` and scanned to
//! `C:\MineRust-data\traces\window-<name>-scan.txt`; the repository keeps
//! only this source.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use parity::{
    copy_dir_all, load_parity_env, run_walk, ParityEnv, RunStats, Stop, Vanilla, WalkMode,
};

const TRACE_DIR: &str = r"C:\MineRust-data\traces";
const WORLD_ROOT: &str = r"C:\MineRust-data";
/// The frozen v3 world: 65×65 full chunks (x −26..38, z −34..30). ЧАСТЬ 3
/// никогда не изменяет его — только копирует под прогон.
const V3_TEMPLATE: &str = r"C:\MineRust-data\world-template-v3";
/// Idle after the last step (Part I kept 5 s).
const SETTLE: Duration = Duration::from_secs(5);

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

/// The probe name and the mode it repeats.
fn probe(name: &str) -> Option<(WalkMode, &'static str)> {
    match name {
        "p2" => Some((
            WalkMode::WalkLoaded {
                diagonal: false,
                steps: 302,
                settle: SETTLE,
                on_ground: true,
                alternate_rot: false,
                tick_coupled: true,
            },
            "П2: straight +x, 302 steps",
        )),
        "p3" => Some((
            WalkMode::WalkLoaded {
                diagonal: true,
                steps: 427,
                settle: SETTLE,
                on_ground: true,
                alternate_rot: false,
                tick_coupled: true,
            },
            "П3: diagonal +x/+z, 427 steps",
        )),
        // ЧАСТЬ 3 раунда 5: 600 steps × 0.2 = ~120 blocks on the frozen v3
        // world, same bot scheme as П2/П3 (tick_coupled, 50 ms deadline grid).
        "p4" => Some((
            WalkMode::WalkV3 {
                diagonal: false,
                steps: 600,
                settle: SETTLE,
            },
            "П4: straight +x, 600 steps (~120 blocks) on v3",
        )),
        "p5" => Some((
            WalkMode::WalkV3 {
                diagonal: true,
                steps: 600,
                settle: SETTLE,
            },
            "П5: diagonal +x/+z, 600 steps (~120 blocks) on v3",
        )),
        _ => None,
    }
}

fn scan(world: &Path, out: &Path) {
    let result = Command::new("python")
        .arg(chunk_status_script())
        .arg("scan")
        .arg(world)
        .output();
    match result {
        Ok(output) => {
            let _ = std::fs::write(out, &output.stdout);
            eprintln!("scan: {} ({} bytes)", out.display(), output.stdout.len());
        }
        Err(error) => eprintln!("scan failed: {error}"),
    }
}

fn summary(name: &str, label: &str, stats: &RunStats, world: &Path, scan_path: &Path) -> String {
    let mut out = String::new();
    out.push_str(&format!("# window probe {name} — {label}\n\n"));
    out.push_str(&format!("- stop: {:?}\n", stats.stop));
    out.push_str(&format!(
        "- moves {}  ticks {}  chunks {}  elapsed {:.0} ms\n",
        stats.moves,
        stats
            .counts
            .get("serverbound / minecraft:client_tick_end")
            .copied()
            .unwrap_or(0),
        stats.chunks,
        stats.elapsed_ms
    ));
    out.push_str(&format!(
        "- player_position: {}  game_mode: {:?}  abilities: {:?}\n",
        stats.teleports, stats.game_mode, stats.abilities_flags
    ));
    out.push_str(&format!(
        "- centres: {}  forgets: {}  batches: {}\n",
        stats
            .cache_moves
            .iter()
            .filter(|m| m.name.ends_with("set_chunk_cache_center"))
            .count(),
        stats
            .cache_moves
            .iter()
            .filter(|m| m.name.ends_with("forget_level_chunk"))
            .count(),
        stats.batches.len()
    ));
    out.push_str(&format!(
        "- world: {}  scan: {}\n",
        world.display(),
        scan_path.display()
    ));
    out
}

#[tokio::test]
#[ignore = "requires the owner's installation (C:\\MineRust-data)"]
async fn window_probe() {
    let env = match parity_env_or_skip() {
        Some(env) => env,
        None => return,
    };
    let name = std::env::var("MINECRAFT_WINDOW_PROBE").unwrap_or_else(|_| "p2".to_owned());
    let Some((mode, label)) = probe(&name) else {
        eprintln!("SKIP: MINECRAFT_WINDOW_PROBE must be p2 or p3, got {name}");
        return;
    };
    let ids = Arc::new(env.packet_ids().expect("packet ids"));
    std::fs::create_dir_all(TRACE_DIR).expect("traces dir");

    // ЧАСТЬ 3: П4/П5 start on a fresh copy of the frozen v3 world; П2/П3 keep
    // the default v2 template. `start_from` copies the template, so the
    // template itself is never touched.
    let template = if name == "p4" || name == "p5" {
        Path::new(V3_TEMPLATE)
    } else {
        &env.world_template
    };
    let server = match Vanilla::start_from(&env, template).await {
        Ok(server) => server,
        Err(error) => {
            eprintln!("vanilla did not start: {error}");
            return;
        }
    };
    let trace = Path::new(TRACE_DIR).join(format!("window-{name}.jsonl"));
    let outcome = run_walk(&env, Arc::clone(&ids), mode, &trace).await;
    if let Err(error) = server.stop_with_flush().await {
        eprintln!("vanilla did not stop cleanly: {error}");
    }

    let stats = match outcome {
        Ok(stats) => stats,
        Err(error) => {
            eprintln!("walk failed: {error}");
            panic!("walk {name} failed: {error}");
        }
    };
    let world = Path::new(WORLD_ROOT).join(format!("world-window-{name}"));
    if world.exists() {
        std::fs::remove_dir_all(&world).expect("remove old world copy");
    }
    copy_dir_all(&env.vanilla_dir.join("world"), &world).expect("copy world");
    let scan_path = Path::new(TRACE_DIR).join(format!("window-{name}-scan.txt"));
    scan(&world, &scan_path);

    let report = summary(&name, label, &stats, &world, &scan_path);
    let _ = std::fs::write(
        Path::new(TRACE_DIR).join(format!("window-{name}.report.md")),
        &report,
    );
    eprintln!("{report}");

    assert_eq!(stats.stop, Stop::Completed, "run must complete, not stop");
    assert_eq!(
        stats.teleports, 1,
        "no rollback: exactly one player_position"
    );
}
