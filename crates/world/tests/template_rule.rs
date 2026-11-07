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

use world::{load_region, Chunk, ChunkNotReady};

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
