//! Шаг 22b release measurements on the real configured v3 template.
//!
//! Run manually; the test needs the owner's generated reports/client jar and
//! world template, so it is ignored in the workspace gate:
//!
//! ```text
//! cargo test -p parity --release --test worldsave_measure -- --ignored --nocapture
//! ```
//!
//! It measures the three requested boundaries: the tick-side snapshot of one
//! dirty v3 chunk, one atomic `.mrchunk` write, and a second socket client
//! entering play after the v3 batch was warmed once.

#[path = "common/twobot.rs"]
#[allow(dead_code)]
mod twobot;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use parity::load_parity_env;
use sha2::{Digest, Sha256};
use twobot::{Behavior, Cmd, End, BOT_A, BOT_B};

#[derive(Debug)]
struct MeasureBehavior {
    joined: Arc<AtomicBool>,
    stop_when: Arc<AtomicBool>,
    mark_on_loaded: Option<Arc<AtomicBool>>,
}

impl Behavior for MeasureBehavior {
    fn on_joined(&mut self) {
        self.joined.store(true, Ordering::SeqCst);
    }

    fn on_teleport(&mut self, _pos: [f64; 3]) {}

    fn on_packet(&mut self, _entry: &parity::TraceEntry, _payload: &[u8]) -> Vec<Cmd> {
        Vec::new()
    }

    fn on_tick(&mut self, loaded: bool, _pos: Option<[f64; 3]>) -> Vec<Cmd> {
        if loaded {
            if let Some(done) = &self.mark_on_loaded {
                done.store(true, Ordering::SeqCst);
            }
        }
        if self.stop_when.load(Ordering::SeqCst) {
            vec![Cmd::Stop("measurement complete".to_owned())]
        } else {
            Vec::new()
        }
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "minerust-worldsave-v3-{name}-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn identity(world_dir: &Path) -> worldsave::WorldIdentity {
    let level_dat = world_dir.join("level.dat");
    let bytes = std::fs::read(&level_dat).expect("read level.dat");
    let level = world::read_level_dat(&level_dat).expect("parse level.dat");
    let level_dat_sha256 = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    worldsave::WorldIdentity {
        template: world_dir.display().to_string(),
        level_dat_sha256,
        data_version: level.data_version,
    }
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

async fn wait_flag(flag: &AtomicBool, what: &str) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while !flag.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the owner's v3 template, generated reports and client jar"]
async fn v3_snapshot_write_and_second_join() {
    let env = Arc::new(load_parity_env().expect("parity environment"));
    let config = env.config.clone();
    let world_dir = Path::new(
        config
            .world_dir
            .as_deref()
            .expect("server.toml has world_dir"),
    );
    let save_dir = temp_dir("save");
    let traces = temp_dir("traces");
    let disk = worldsave::Store::open(&save_dir, &identity(world_dir), false)
        .expect("open temporary world save");
    let disk = Arc::new(disk);
    let ids = Arc::new(env.packet_ids().expect("packet ids"));
    let data = Arc::new(
        data::load_from_jar(
            &config.client_jar,
            config.registries_report.as_deref().map(Path::new),
        )
        .expect("game data"),
    );
    let (loader, world) =
        server::build_world_with_save(&config, Arc::clone(&data), Some(Arc::clone(&disk)))
            .expect("v3 world builds");
    let loader = loader.expect("configured world has a loader");
    let reply = loader.load_batch().await.expect("warm-up queued");
    let ready = reply
        .await
        .expect("warm-up reply")
        .expect("v3 batch builds");

    // Change one real v3 cell to create exactly one dirty chunk. The value is
    // deliberately picked from the generated report, never hard-coded.
    let position = world.spawn;
    let y = position.y - 1;
    let old = ready
        .block(position.x, y, position.z)
        .expect("spawn chunk cell");
    let air = ready
        .blocks()
        .default_id("minecraft:air")
        .expect("air in generated block report");
    let stone = ready
        .blocks()
        .default_id("minecraft:stone")
        .expect("stone in generated block report");
    let replacement = if old == stone { air } else { stone };
    ready
        .set_block(position.x, y, position.z, replacement)
        .expect("mark one v3 chunk dirty");

    let mut snapshots = Vec::with_capacity(500);
    for _ in 0..500 {
        let started = Instant::now();
        let batch = ready.autosave_batch(256);
        snapshots.push(started.elapsed());
        assert_eq!(batch.len(), 1);
        std::hint::black_box(&batch);
        ready.not_queued(&batch);
    }
    let one = ready.autosave_batch(256).remove(0);
    let diff = worldsave::ChunkDiff {
        chunk_x: one.chunk_x,
        chunk_z: one.chunk_z,
        changes: one.changes,
    };
    let mut writes = Vec::with_capacity(30);
    for _ in 0..30 {
        let started = Instant::now();
        disk.write_diff(&diff).expect("atomic v3 diff write");
        writes.push(started.elapsed());
    }
    disk.write_diff(&worldsave::ChunkDiff {
        chunk_x: diff.chunk_x,
        chunk_z: diff.chunk_z,
        changes: Vec::new(),
    })
    .expect("remove measurement file");
    ready.reset_dirty();

    let (runtime, writer) =
        worldsave::spawn((*disk).clone(), worldsave::DEFAULT_CAPACITY).expect("world-save writer");
    let runtime = Arc::new(runtime);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let port = listener.local_addr().expect("local address").port();
    let game_config = net::game::GameConfig {
        workers: config.workers,
        region_side: config.region_side,
        default_game_mode: 3,
        store: Some(Arc::clone(&ready)),
        worldsave: Some(Arc::clone(&runtime) as Arc<dyn net::game::WorldSave>),
        world_autosave_ticks: 0,
        ..net::game::GameConfig::default()
    };
    let server = tokio::spawn(net::serve_with_metrics(
        game_config,
        None,
        listener,
        Arc::new(server::status_info(&config)),
        data,
        Arc::clone(&ids),
        Some(loader),
        world,
    ));

    let a_joined = Arc::new(AtomicBool::new(false));
    let b_joined = Arc::new(AtomicBool::new(false));
    let b_loaded = Arc::new(AtomicBool::new(false));
    let stop_a = Arc::new(AtomicBool::new(false));
    let stop_b = Arc::new(AtomicBool::new(false));
    let a = tokio::spawn(twobot::run_bot_at(
        Arc::clone(&env),
        Arc::clone(&ids),
        port,
        BOT_A,
        traces.join("measure-A.jsonl"),
        Duration::from_secs(90),
        MeasureBehavior {
            joined: Arc::clone(&a_joined),
            stop_when: Arc::clone(&stop_a),
            mark_on_loaded: None,
        },
    ));
    wait_flag(&a_joined, "the first client to enter play").await;

    let second_started = Instant::now();
    let b = tokio::spawn(twobot::run_bot_at(
        Arc::clone(&env),
        Arc::clone(&ids),
        port,
        BOT_B,
        traces.join("measure-B.jsonl"),
        Duration::from_secs(90),
        MeasureBehavior {
            joined: Arc::clone(&b_joined),
            stop_when: Arc::clone(&stop_b),
            mark_on_loaded: Some(Arc::clone(&b_loaded)),
        },
    ));
    wait_flag(&b_joined, "the second client to enter play").await;
    let second_join = second_started.elapsed();
    wait_flag(&b_loaded, "the second client player_loaded tick").await;
    stop_a.store(true, Ordering::SeqCst);
    stop_b.store(true, Ordering::SeqCst);
    let (a_report, b_report) = (a.await.expect("A task"), b.await.expect("B task"));
    assert!(
        matches!(a_report.end, End::Script(_)),
        "A: {}",
        a_report.end.text()
    );
    assert!(
        matches!(b_report.end, End::Script(_)),
        "B: {}",
        b_report.end.text()
    );

    println!(
        "v3 dirty snapshot median: {:.1} us",
        median(snapshots).as_secs_f64() * 1e6
    );
    println!(
        "v3 atomic diff write median: {:.2} ms",
        median(writes).as_secs_f64() * 1e3
    );
    println!(
        "v3 second client enters play: {:.2} ms",
        second_join.as_secs_f64() * 1e3
    );

    server.abort();
    let _ = server.await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    writer
        .shutdown(Duration::from_secs(5))
        .expect("writer stops");
    drop(runtime);
    let _ = std::fs::remove_dir_all(save_dir);
    let _ = std::fs::remove_dir_all(traces);
}
