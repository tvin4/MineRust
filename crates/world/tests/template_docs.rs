//! Audit of the saved NBT documents of the owner's real world template.
//!
//! Every `.dat` the game writes is opened with the very readers the server
//! uses (plus the generic [`world::read_world_document`] for the documents
//! no code path consumes yet) and the key fields are checked against the
//! values walked byte by byte on the owner's machine:
//!
//! * `level.dat` — `Data.DataVersion` 5023 and `Data.spawn.pos`
//!   `[96, 145, -32]`;
//! * `data/minecraft/world_gen_settings.dat` — `data.seed` 12345 (the
//!   step 3.3 blocker: the seed inside the `data` envelope);
//! * `data/minecraft/game_rules.dat` — decodes, with observed rules
//!   (`max_command_forks`, `keep_inventory`) at their keys;
//! * every other saved document — decodes to a compound, so a format the
//!   decoder does not know fails here instead of at a first live join.
//!
//! The synthetic fixtures next to the readers mirror these real shapes
//! (see `level.rs` and `server::tests`).
//!
//! Run with the template directory set, like `tests/template_rule.rs`:
//!
//! ```text
//! MINECRAFT_WORLD_TEMPLATE=C:\MineRust-data\world-template \
//!   cargo test -p world --test template_docs -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};

use protocol::{NbtTag, Position};
use world::{read_level_dat, read_world_document, read_world_gen_settings};

/// `DataVersion` observed in the template's `level.dat`.
const TEMPLATE_DATA_VERSION: i32 = 5023;

/// Spawn block observed in the template's `level.dat`.
const TEMPLATE_SPAWN: Position = Position {
    x: 96,
    y: 145,
    z: -32,
};

/// Seed observed in the template's `world_gen_settings.dat`.
const TEMPLATE_SEED: i64 = 12345;

/// Directory of the frozen template; the same variable as `template_rule`.
fn template_dir() -> Option<PathBuf> {
    let dir = std::env::var("MINECRAFT_WORLD_TEMPLATE")
        .ok()
        .map(PathBuf::from)?;
    if dir.is_dir() {
        return Some(dir);
    }
    eprintln!(
        "SKIP: MINECRAFT_WORLD_TEMPLATE is not a directory: {}",
        dir.display()
    );
    None
}

/// Every saved `.dat` under the template, in path order.
fn dat_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect(dir, &mut files);
    files.sort();
    files
}

/// One directory level of the collection above.
fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "dat") {
            files.push(path);
        }
    }
}

/// A `data`-enveloped child of one document (the shape every saved
/// data-pack document shares: `{data:{...}}`).
fn under_data<'a>(document: &'a NbtTag, key: &str) -> Option<&'a NbtTag> {
    let NbtTag::Compound(root) = document else {
        return None;
    };
    match root.get("data")? {
        NbtTag::Compound(cell) => cell.get(key),
        _ => None,
    }
}

/// Step 3.3 regression: the real `level.dat` through the real reader.
#[test]
#[ignore = "requires the owner's world template (MINECRAFT_WORLD_TEMPLATE)"]
fn the_level_dat_of_the_template_reads_version_and_spawn() {
    let Some(dir) = template_dir() else {
        return;
    };
    let level = read_level_dat(dir.join("level.dat")).unwrap_or_else(|error| {
        panic!("the template's level.dat must read: {error}");
    });
    assert_eq!(
        level.data_version, TEMPLATE_DATA_VERSION,
        "Data.DataVersion of the template"
    );
    assert_eq!(
        level.spawn, TEMPLATE_SPAWN,
        "Data.spawn.pos of the template"
    );
}

/// The step 3.3 blocker fix against the very file that exposed it: the
/// seed sits inside `data`, not at the root.
#[test]
#[ignore = "requires the owner's world template (MINECRAFT_WORLD_TEMPLATE)"]
fn the_seed_of_the_template_comes_from_inside_the_data_envelope() {
    let Some(dir) = template_dir() else {
        return;
    };
    let path = dir.join("data/minecraft/world_gen_settings.dat");
    let settings = read_world_gen_settings(&path)
        .unwrap_or_else(|error| panic!("the template's seed must read: {error}"));
    assert_eq!(
        settings.seed, TEMPLATE_SEED,
        "data.seed of the template's generation settings"
    );
}

/// Every saved document decodes, and the two documents the audit names
/// hold their observed key fields where the readers look for them.
#[test]
#[ignore = "requires the owner's world template (MINECRAFT_WORLD_TEMPLATE)"]
fn every_saved_document_of_the_template_decodes_with_its_key_fields() {
    let Some(dir) = template_dir() else {
        return;
    };
    let files = dat_files(&dir);
    assert!(!files.is_empty(), "the template holds saved documents");

    let mut decoded = Vec::new();
    for path in &files {
        let document = read_world_document(path).unwrap_or_else(|error| {
            panic!("{} must decode: {error}", path.display());
        });
        assert!(
            matches!(document, NbtTag::Compound(_)),
            "{}: root must be a compound",
            path.display()
        );
        decoded.push((path.clone(), document));
    }
    eprintln!("decoded {} saved documents", decoded.len());

    // game_rules.dat: the envelope plus rules observed on this machine.
    let rules = decoded
        .iter()
        .find(|(path, _)| path.ends_with("data/minecraft/game_rules.dat"))
        .map(|(_, document)| document)
        .expect("game_rules.dat is part of the audit");
    match under_data(rules, "minecraft:max_command_forks") {
        Some(NbtTag::Int(value)) => assert_eq!(*value, 65536, "observed rule"),
        other => panic!("max_command_forks int under data, got {other:?}"),
    }
    match under_data(rules, "minecraft:keep_inventory") {
        Some(NbtTag::Byte(value)) => assert_eq!(*value, 0, "observed rule"),
        other => panic!("keep_inventory byte under data, got {other:?}"),
    }

    // world_gen_settings.dat: the seed the server's reader now takes.
    let settings = decoded
        .iter()
        .find(|(path, _)| path.ends_with("data/minecraft/world_gen_settings.dat"))
        .map(|(_, document)| document)
        .expect("world_gen_settings.dat is part of the audit");
    match under_data(settings, "seed") {
        Some(NbtTag::Long(value)) => assert_eq!(*value, TEMPLATE_SEED, "data.seed"),
        other => panic!("seed long under data, got {other:?}"),
    }

    // level.dat: its envelope is read directly by `read_level_dat`.
    let level = decoded
        .iter()
        .find(|(path, _)| path.ends_with("level.dat"))
        .map(|(_, document)| document)
        .expect("level.dat is part of the audit");
    assert!(
        matches!(level, NbtTag::Compound(root) if root.contains_key("Data")),
        "level.dat holds the Data compound"
    );
}
