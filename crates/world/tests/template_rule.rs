//! Reads the owner's world template and applies the send rule to it.
//!
//! `MINECRAFT_WORLD_TEMPLATE=path cargo test -p world --test template_rule
//! -- --ignored --nocapture`
//!
//! The expectations below are the step 3.0 observations (nine ready chunks
//! of 529, only they carrying light): an empty or freshly generated world
//! must fail here rather than silently send something else.

use std::collections::BTreeMap;
use std::path::PathBuf;

use world::{load_region, load_sendable_chunk, Chunk, ChunkNotReady, ChunkUnavailable};

/// Directory of the frozen template; normally the one configured for parity.
fn template_dir() -> Option<PathBuf> {
    let dir = std::env::var("MINECRAFT_WORLD_TEMPLATE")
        .ok()
        .map(PathBuf::from)?;
    if dir.is_dir() {
        Some(dir)
    } else {
        eprintln!(
            "SKIP: MINECRAFT_WORLD_TEMPLATE is not a directory: {}",
            dir.display()
        );
        None
    }
}

/// Region files of the overworld of the template, in name order.
fn region_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let region = dir.join("dimensions/minecraft/overworld/region");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&region)
        .expect("region directory of the template")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "mca"))
        .collect();
    files.sort();
    files
}

#[test]
#[ignore = "requires the owner's world template (MINECRAFT_WORLD_TEMPLATE)"]
fn the_send_rule_matches_the_observed_template() {
    let dir = match template_dir() {
        Some(dir) => dir,
        None => return,
    };
    let data_version = 5023; // level.dat DataVersion of the template.

    let mut ready: Vec<(i32, i32)> = Vec::new();
    let mut ready_statuses: BTreeMap<String, usize> = BTreeMap::new();
    let mut refused: Vec<ChunkNotReady> = Vec::new();
    let mut statuses: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0_usize;
    for file in region_files(&dir) {
        for stored in load_region(&file).expect("region must read") {
            let chunk = Chunk::from_stored(&stored).expect("model of a stored chunk");
            total += 1;
            match chunk.check_sendable(data_version) {
                Ok(()) => {
                    ready.push((chunk.x, chunk.z));
                    *ready_statuses.entry(chunk.status.clone()).or_default() += 1;
                }
                Err(reason) => refused.push(reason),
            }
        }
    }

    eprintln!("total chunks: {total}");
    eprintln!("sendable: {}", ready.len());
    eprintln!("refused: {}", refused.len());
    eprintln!("sendable positions: {ready:?}");

    assert_eq!(total, 529, "template must hold the observed 529 chunks");
    assert_eq!(
        ready.len(),
        9,
        "only the nine light-carrying chunks are sendable"
    );
    // The nine are the 3×3 square around the spawn chunk (6, -2).
    let mut sorted = ready.clone();
    sorted.sort();
    let expected = [
        (5, -3),
        (5, -2),
        (5, -1),
        (6, -3),
        (6, -2),
        (6, -1),
        (7, -3),
        (7, -2),
        (7, -1),
    ];
    assert_eq!(sorted, expected, "the ready square is the observed one");

    // Refusals must all carry a data-driven reason; the counts of status
    // reasons repeat the step 3.0 inventory (no default reasons).
    for reason in &refused {
        match reason {
            ChunkNotReady::NotGenerated { status, .. } => {
                *statuses.entry(status.clone()).or_default() += 1
            }
            other => panic!("unexpected refusal reason: {other}"),
        }
    }
    assert_eq!(statuses.get("minecraft:structure_starts"), Some(&480));
    assert_eq!(statuses.get("minecraft:biomes"), Some(&24));
    assert_eq!(statuses.get("minecraft:terrain"), Some(&16));
    // The sendable nine are the eight `initialize_light` plus the one `full`
    // chunk; neither ever reaches the refusal list.
    assert_eq!(ready_statuses.get("minecraft:initialize_light"), Some(&8));
    assert_eq!(ready_statuses.get("minecraft:full"), Some(&1));
    eprintln!("sendable status counts: {ready_statuses:?}");
    eprintln!("refused status counts: {statuses:?}");
}

/// The same rule through the coordinate lookup the loader will call
/// (step 3.3b): the nine observed chunks must be found and accepted by
/// name, and every refusal must stay a specific, explicit value.
#[test]
#[ignore = "requires the owner's world template (MINECRAFT_WORLD_TEMPLATE)"]
fn the_lookup_gate_answers_by_coordinates_with_reasons() {
    let dir = match template_dir() {
        Some(dir) => dir,
        None => return,
    };
    let region = dir.join("dimensions/minecraft/overworld/region");
    let data_version = 5023; // level.dat DataVersion of the template.

    let ready = [
        (5, -3),
        (5, -2),
        (5, -1),
        (6, -3),
        (6, -2),
        (6, -1),
        (7, -3),
        (7, -2),
        (7, -1),
    ];
    for (x, z) in ready {
        let chunk = load_sendable_chunk(&region, x, z, data_version)
            .unwrap_or_else(|err| panic!("chunk ({x}, {z}) is under the rule: {err}"));
        assert_eq!((chunk.x, chunk.z), (x, z), "the chunk of these coordinates");
    }

    // Outside the template the gate says "not stored", never a default chunk.
    let error = load_sendable_chunk(&region, 40_000, 40_000, data_version)
        .expect_err("no region file is that far out");
    assert!(
        matches!(
            error,
            ChunkUnavailable::NotStored {
                x: 40_000,
                z: 40_000
            }
        ),
        "{error}"
    );

    // The very chunk the rule accepts under this world's schema keeps
    // refusing under another one, by name and with both versions.
    let error = load_sendable_chunk(&region, 6, -2, 9999).expect_err("another DataVersion");
    match error {
        ChunkUnavailable::NotReady(ChunkNotReady::UnsupportedVersion {
            x: 6,
            z: -2,
            found: 5023,
            expected: 9999,
        }) => {}
        other => panic!("expected an explicit version refusal, got {other}"),
    }

    // A chunk of the 529 that waits for generation names its stored status
    // through the lookup too (as opposed to never being found).
    let sample = (0, 0);
    let error = load_sendable_chunk(&region, sample.0, sample.1, data_version)
        .expect_err("not every chunk of the template is sendable");
    match &error {
        ChunkUnavailable::NotReady(ChunkNotReady::NotGenerated { status, .. }) => {
            assert!(status.starts_with("minecraft:"), "{error}");
        }
        other => panic!("expected a stored status refusal, got {other}"),
    }
    eprintln!("lookup refusal: {error}");
}
