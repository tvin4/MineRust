//! Tests of the reverse lookup of the registries report (Шаг 24d).

use super::registries_from_json;

/// A report slice with the two registries the inventory touches.
fn report() -> &'static str {
    r#"{
      "minecraft:item": {"entries": {
        "minecraft:stone": {"protocol_id": 1},
        "minecraft:oak_log": {"protocol_id": 163},
        "minecraft:oak_stairs": {"protocol_id": 515}
      }},
      "minecraft:block": {"entries": {
        "minecraft:stone": {"protocol_id": 1},
        "minecraft:oak_log": {"protocol_id": 137}
      }}
    }"#
}

#[test]
fn an_id_maps_back_to_its_entry_name() {
    let ids = registries_from_json(report()).expect("valid report");

    assert_eq!(
        ids.entry_id("minecraft:item", "minecraft:oak_log"),
        Some(163)
    );
    assert_eq!(
        ids.entry_name("minecraft:item", 163),
        Some("minecraft:oak_log")
    );
    assert_eq!(
        ids.entry_name("minecraft:block", 137),
        Some("minecraft:oak_log")
    );
    // A block and an item share an identifier but not an id: the registries
    // stay separate, which is why the item lookup names its registry.
    assert_eq!(ids.entry_name("minecraft:block", 163), None);
    assert_eq!(ids.entry_name("minecraft:item", 0), None);
    assert_eq!(ids.entry_name("minecraft:none", 1), None);
}
