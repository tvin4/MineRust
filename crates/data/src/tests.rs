//! Tests on self-made fixtures only: every jar used here is built in
//! memory from files defined in this file. No game data is stored in the
//! repository. The real client jar is covered by one `#[ignore]`d test
//! that runs when `MINECRAFT_JAR` points at it.

use std::io::{Cursor, Write};

use protocol::NbtTag;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::load::JAR_DATA_PREFIX;
use crate::{load_from_jar, load_from_zip, DataError, GameData, SYNCHRONIZED_REGISTRIES};

/// Builds an in-memory jar from `files`, plus a placeholder entry for every
/// synchronized registry the test does not fill itself (each registry must
/// be non-empty).
fn jar_with(files: &[(&str, &str)]) -> Cursor<Vec<u8>> {
    let mut all: Vec<(String, String)> = files
        .iter()
        .map(|(name, content)| ((*name).to_owned(), (*content).to_owned()))
        .collect();
    for dir in SYNCHRONIZED_REGISTRIES {
        let prefix = format!("{JAR_DATA_PREFIX}{dir}/");
        if !all.iter().any(|(name, _)| name.starts_with(&prefix)) {
            all.push((format!("{prefix}fixture.json"), "{}".to_owned()));
        }
    }
    write_jar(&all)
}

/// Builds an in-memory jar with exactly the given files.
fn jar_exactly(files: &[(&str, &str)]) -> Cursor<Vec<u8>> {
    let owned: Vec<(String, String)> = files
        .iter()
        .map(|(name, content)| ((*name).to_owned(), (*content).to_owned()))
        .collect();
    write_jar(&owned)
}

fn write_jar(files: &[(String, String)]) -> Cursor<Vec<u8>> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in files {
        writer
            .start_file(name.as_str(), SimpleFileOptions::default())
            .expect("fixture entry");
        writer
            .write_all(content.as_bytes())
            .expect("fixture content");
    }
    writer.finish().expect("fixture jar")
}

/// Loads a fixture jar built from `files`.
fn load(files: &[(&str, &str)]) -> Result<GameData, DataError> {
    load_from_zip(jar_with(files))
}

#[test]
fn every_synchronized_registry_is_loaded() {
    let data = load(&[]).expect("fixture loads");
    assert_eq!(data.registries().count(), SYNCHRONIZED_REGISTRIES.len());
    for dir in SYNCHRONIZED_REGISTRIES {
        let id = format!("minecraft:{dir}");
        let registry = data.registry(&id).expect("registry present");
        assert_eq!(registry.entries().len(), 1, "{id}");
    }
}

#[test]
fn registry_entries_are_sorted_and_namespaced() {
    // Deliberately out of order: ids must follow the sorted order.
    let data = load(&[
        (
            "data/minecraft/damage_type/generic.json",
            r#"{"message_id":"generic"}"#,
        ),
        (
            "data/minecraft/damage_type/fall.json",
            r#"{"message_id":"fall"}"#,
        ),
    ])
    .expect("fixture loads");
    let registry = data.registry("minecraft:damage_type").expect("exists");
    let names: Vec<&str> = registry
        .entries()
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, ["minecraft:fall", "minecraft:generic"]);
    assert_eq!(registry.entry_id("minecraft:fall"), Some(0));
    assert_eq!(registry.entry_id("minecraft:generic"), Some(1));
    assert_eq!(registry.entry_id("minecraft:nope"), None);
    // Names greater than every entry must not panic the binary search.
    assert_eq!(registry.entry_id("minecraft:zzz"), None);
}

#[test]
fn registry_entry_data_is_json_converted_to_nbt() {
    let data = load(&[(
        "data/minecraft/dimension_type/overworld.json",
        r#"{"height": 384, "has_skylight": true, "name": "overworld"}"#,
    )])
    .expect("fixture loads");
    let registry = data.registry("minecraft:dimension_type").expect("exists");
    let entry = &registry.entries()[0];
    let NbtTag::Compound(fields) = &entry.data else {
        panic!("expected a compound, got {:?}", entry.data);
    };
    assert_eq!(fields["height"], NbtTag::Short(384));
    assert_eq!(fields["has_skylight"], NbtTag::Byte(1));
    assert_eq!(fields["name"], NbtTag::String("overworld".to_owned()));
}

#[test]
fn a_registry_without_entries_is_an_error() {
    let err = load_from_zip(jar_exactly(&[(
        "data/minecraft/damage_type/fall.json",
        "{}",
    )]))
    .expect_err("the other registries are missing");
    assert!(matches!(err, DataError::MissingRegistry { .. }), "{err}");
}

#[test]
fn nested_registry_files_are_rejected() {
    let err =
        load(&[("data/minecraft/damage_type/sub/x.json", "{}")]).expect_err("entries must be flat");
    assert!(matches!(err, DataError::NestedEntry { .. }), "{err}");
}

#[test]
fn tag_references_expand_to_numeric_ids() {
    let data = load(&[
        (
            "data/minecraft/damage_type/fire.json",
            r#"{"message_id":"fire"}"#,
        ),
        (
            "data/minecraft/damage_type/fall.json",
            r#"{"message_id":"fall"}"#,
        ),
        (
            "data/minecraft/tags/damage_type/is_fire.json",
            r#"{"values":["minecraft:fire"]}"#,
        ),
        (
            "data/minecraft/tags/damage_type/is_damage.json",
            r##"{"values":["#minecraft:is_fire","minecraft:fall"]}"##,
        ),
    ])
    .expect("fixture loads");
    let registry = data.registry("minecraft:damage_type").expect("exists");
    let fire = registry.entry_id("minecraft:fire").expect("entry");
    let fall = registry.entry_id("minecraft:fall").expect("entry");
    let damage = &data.tags()["minecraft:damage_type"]["minecraft:is_damage"];
    // The `#minecraft:is_fire` reference expands in place, then `fall`.
    assert_eq!(damage, &[fire, fall]);
}

#[test]
fn tags_of_unsynchronized_registries_are_ignored() {
    let data = load(&[(
        "data/minecraft/tags/block/logs.json",
        r#"{"values":["minecraft:stone"]}"#,
    )])
    .expect("fixture loads");
    // `block` is a built-in registry: its tags stay with the client.
    assert!(data.tags().is_empty(), "{:?}", data.tags());
}

#[test]
fn nested_tag_files_use_the_full_tag_path() {
    let data = load(&[
        (
            "data/minecraft/banner_pattern/skull.json",
            r#"{"asset_id":"minecraft:skull"}"#,
        ),
        (
            "data/minecraft/tags/banner_pattern/pattern_item/skull.json",
            r#"{"values":["minecraft:skull"]}"#,
        ),
    ])
    .expect("fixture loads");
    let banner_tags = &data.tags()["minecraft:banner_pattern"];
    let registry = data.registry("minecraft:banner_pattern").expect("exists");
    let skull = registry.entry_id("minecraft:skull").expect("entry");
    assert_eq!(banner_tags["minecraft:pattern_item/skull"], [skull]);
}

#[test]
fn missing_required_references_are_an_error() {
    let err = load(&[
        ("data/minecraft/damage_type/fall.json", "{}"),
        (
            "data/minecraft/tags/damage_type/bad.json",
            r#"{"values":["minecraft:nope"]}"#,
        ),
    ])
    .expect_err("the entry does not exist");
    assert!(matches!(err, DataError::MissingEntry { .. }), "{err}");

    let err = load(&[(
        "data/minecraft/tags/damage_type/a.json",
        r##"{"values":["#minecraft:nope"]}"##,
    )])
    .expect_err("the tag does not exist");
    assert!(matches!(err, DataError::MissingTag { .. }), "{err}");
}

#[test]
fn missing_optional_references_are_skipped() {
    let data = load(&[
        ("data/minecraft/damage_type/fall.json", "{}"),
        (
            "data/minecraft/tags/damage_type/maybe.json",
            r#"{"values":[{"id":"minecraft:nope","required":false},"minecraft:fall"]}"#,
        ),
        (
            "data/minecraft/tags/damage_type/only_optional.json",
            r##"{"replace":true,"values":[{"id":"minecraft:nope","required":false}]}"##,
        ),
    ])
    .expect("fixture loads");
    let damage_tags = &data.tags()["minecraft:damage_type"];
    let registry = data.registry("minecraft:damage_type").expect("exists");
    let fall = registry.entry_id("minecraft:fall").expect("entry");
    assert_eq!(damage_tags["minecraft:maybe"], [fall]);
    // A tag with no entries left is not sent at all.
    assert!(!damage_tags.contains_key("minecraft:only_optional"));
}

#[test]
fn tag_cycles_are_an_error() {
    let err = load(&[
        (
            "data/minecraft/tags/damage_type/a.json",
            r##"{"values":["#minecraft:b"]}"##,
        ),
        (
            "data/minecraft/tags/damage_type/b.json",
            r##"{"values":["#minecraft:a"]}"##,
        ),
    ])
    .expect_err("the tags reference each other");
    assert!(matches!(err, DataError::TagCycle { .. }), "{err}");
}

#[test]
fn values_are_deduplicated_and_unqualified_names_default_to_minecraft() {
    let data = load(&[
        ("data/minecraft/damage_type/fall.json", "{}"),
        ("data/minecraft/damage_type/fire.json", "{}"),
        (
            "data/minecraft/tags/damage_type/is_fire.json",
            r#"{"values":["fire"]}"#,
        ),
        (
            "data/minecraft/tags/damage_type/mixed.json",
            r##"{"values":["fall","#is_fire","minecraft:fire","fall"]}"##,
        ),
    ])
    .expect("fixture loads");
    let registry = data.registry("minecraft:damage_type").expect("exists");
    let fall = registry.entry_id("minecraft:fall").expect("entry");
    let fire = registry.entry_id("minecraft:fire").expect("entry");
    let damage_tags = &data.tags()["minecraft:damage_type"];
    // `fall` twice, the reference expanding to `fire`, then `fire` again:
    // every id appears exactly once, in first-seen order.
    assert_eq!(damage_tags["minecraft:mixed"], [fall, fire]);
}

/// Loads the real client jar when `MINECRAFT_JAR` points at one; skipped by
/// `cargo test` because CI has no game data. Run manually with:
/// `MINECRAFT_JAR=path/to/client.jar cargo test -p data -- --ignored`
#[test]
#[ignore = "needs MINECRAFT_JAR pointing at a real client jar"]
fn load_the_real_client_jar() {
    let path = std::env::var("MINECRAFT_JAR")
        .expect("MINECRAFT_JAR must point at a client jar to run this test");
    let data = load_from_jar(path).expect("the jar must load");
    assert_eq!(data.registries().count(), SYNCHRONIZED_REGISTRIES.len());
    for dir in SYNCHRONIZED_REGISTRIES {
        let id = format!("minecraft:{dir}");
        let registry = data.registry(&id).expect("registry present");
        assert!(!registry.entries().is_empty(), "{id}");
    }
    // Tags required to leave the configuration state (see the public
    // protocol documentation) must resolve.
    let damage_tags = &data.tags()["minecraft:damage_type"];
    for tag in [
        "minecraft:bypasses_shield",
        "minecraft:is_explosion",
        "minecraft:is_fire",
    ] {
        assert!(!damage_tags[tag].is_empty(), "{tag}");
    }
    let banner_tags = &data.tags()["minecraft:banner_pattern"];
    for tag in [
        "minecraft:pattern_item/bordure_indented",
        "minecraft:pattern_item/creeper",
        "minecraft:pattern_item/field_masoned",
        "minecraft:pattern_item/flow",
        "minecraft:pattern_item/flower",
        "minecraft:pattern_item/globe",
        "minecraft:pattern_item/guster",
        "minecraft:pattern_item/mojang",
        "minecraft:pattern_item/piglin",
        "minecraft:pattern_item/skull",
    ] {
        assert!(!banner_tags[tag].is_empty(), "{tag}");
    }
}
