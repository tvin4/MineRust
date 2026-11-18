//! Process-level crash check for Шаг 22b.
//!
//! The child queues one real writer batch and waits until the writer reports
//! that `.tmp` + file sync + rename + directory sync completed. The parent
//! then terminates the child without running either shutdown path and reopens
//! the save. This is deliberately separate from the in-process round-trip:
//! destructors cannot make the result pass after the kill.
//!
//! ```text
//! cargo test -p worldsave --release --test kill_after_autosave -- --ignored --nocapture
//! ```

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use net::game::WorldSave as _;
use net::store::ChunkSave;
use worldsave::{Store, WorldIdentity, DEFAULT_CAPACITY};

const CHILD_ENV: &str = "MINERUST_WORLDSAVE_KILL_HELPER";
const ROOT_ENV: &str = "MINERUST_WORLDSAVE_KILL_ROOT";
const READY_FILE: &str = "autosave-complete";

fn identity() -> WorldIdentity {
    WorldIdentity {
        template: "E:/worlds/template".to_owned(),
        level_dat_sha256: "abc".to_owned(),
        data_version: 5023,
    }
}

fn temp_dir() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "minerust-worldsave-kill-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Child entry: write one autosave and stay alive until the parent kills the
/// process. Without the environment flag this ignored helper is a no-op.
#[test]
#[ignore = "helper process for kill_after_a_completed_autosave"]
fn kill_helper_writes_and_waits() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let root = PathBuf::from(std::env::var_os(ROOT_ENV).expect("save root env"));
    let store = Store::open(&root, &identity(), false).expect("open child store");
    let (runtime, _writer) = worldsave::spawn(store, DEFAULT_CAPACITY).expect("spawn writer");
    assert!(runtime.submit(vec![ChunkSave {
        chunk_x: 2,
        chunk_z: -3,
        generation: 7,
        changes: vec![(42, 1)],
    }]));

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if runtime.completed().iter().any(|done| done.ok) {
            fs::write(root.join(READY_FILE), b"durable").expect("write ready marker");
            loop {
                std::thread::park();
            }
        }
        assert!(
            Instant::now() < deadline,
            "writer did not acknowledge autosave"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// A hard process termination after the writer's successful acknowledgement
/// leaves a readable, checksummed diff on disk.
#[test]
#[ignore = "spawns and terminates a helper process"]
fn kill_after_a_completed_autosave_keeps_the_diff() {
    let root = temp_dir();
    let mut child = Command::new(std::env::current_exe().expect("test executable"))
        .arg("--ignored")
        .arg("--exact")
        .arg("kill_helper_writes_and_waits")
        .arg("--nocapture")
        .env(CHILD_ENV, "1")
        .env(ROOT_ENV, &root)
        .spawn()
        .expect("spawn helper");

    wait_for_ready(&root, &mut child);
    child.kill().expect("terminate helper without shutdown");
    let status = child.wait().expect("wait for killed helper");
    assert!(
        !status.success(),
        "the helper must be killed, not exit cleanly"
    );

    let reopened = Store::open(&root, &identity(), false).expect("reopen after kill");
    let diff = reopened
        .read_diff(2, -3)
        .expect("read diff")
        .expect("diff survived");
    assert_eq!(diff.changes, vec![(42, 1)]);
    let _ = fs::remove_dir_all(root);
}

fn wait_for_ready(root: &Path, child: &mut std::process::Child) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if root.join(READY_FILE).is_file() {
            return;
        }
        if let Some(status) = child.try_wait().expect("helper status") {
            panic!("helper exited before its autosave marker: {status}");
        }
        assert!(Instant::now() < deadline, "helper autosave timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}
