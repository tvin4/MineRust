//! Test-only chunks and lookups for the wire conversion.
//!
//! The ids here stand in for the data reports of the user's own game copy:
//! the values (0, 1, 2, 34, ...) mean nothing by themselves — only the
//! mapping between the fixture's names has to agree with what the conversion
//! expects (state ids for palette names, `block_entity_type` ids for entity
//! types, wire positions for biome names).

use std::collections::BTreeMap;

use protocol::NbtTag;

use crate::chunk::{Chunk, Section};
use crate::wire::WireRefs;

/// Section slots of the template's dimension: Y −4 ... 19.
pub(crate) const SECTION_SLOTS: usize = 24;

/// State id of `minecraft:air` in the fixture.
pub(crate) const AIR: i32 = 0;

/// The lookups the conversion asks for, wired to the fixture ids above.
pub(crate) struct TestRefs;

impl WireRefs for TestRefs {
    fn block_state_id(
        &self,
        name: &str,
        properties: Option<&BTreeMap<String, String>>,
    ) -> Option<i32> {
        match name {
            "minecraft:air" => Some(AIR),
            "minecraft:stone" => Some(1),
            "minecraft:water" => Some(2),
            "minecraft:sculk_sensor" => Some(3),
            "minecraft:oak_slab" => slab_id(properties),
            _ => None,
        }
    }

    fn block_entity_type_id(&self, name: &str) -> Option<i32> {
        match name {
            "minecraft:sculk_sensor" => Some(34),
            "minecraft:sculk_catalyst" => Some(36),
            "minecraft:mob_spawner" => Some(9),
            _ => None,
        }
    }

    fn biome_id(&self, name: &str) -> Option<i32> {
        match name {
            "minecraft:plains" => Some(1),
            "minecraft:forest" => Some(2),
            _ => None,
        }
    }
}

/// State ids of the fixture's oak slab: default/bottom = 41, top = 42, any
/// other property combination does not exist (the report's exact match).
fn slab_id(properties: Option<&BTreeMap<String, String>>) -> Option<i32> {
    match properties
        .and_then(|values| values.get("type"))
        .map(String::as_str)
    {
        None | Some("bottom") => Some(41),
        Some("top") => Some(42),
        Some(_) => None,
    }
}

/// The fixture base: chunk (6, −2) with the three stored heightmaps, every
/// section a single-value palette (air blocks, plains biomes) and no light.
pub(crate) fn chunk() -> Chunk {
    Chunk {
        x: 6,
        z: -2,
        data_version: 5023,
        status: "minecraft:full".to_owned(),
        sections: sections(),
        root: NbtTag::Compound(BTreeMap::from([
            ("Heightmaps".to_owned(), heightmaps()),
            ("block_entities".to_owned(), NbtTag::List(Vec::new())),
        ])),
    }
}

/// The base chunk with a packed bottom section: the block palette
/// [air, stone, water] holds one stone (cell 5) and one water (cell 4000),
/// the biome palette [plains, forest] packs two biomes at one bit each.
pub(crate) fn packed_chunk() -> Chunk {
    let mut chunk = chunk();
    chunk.sections[0].block_states = Some(container(block_palette(), bottom_data(false)));
    chunk.sections[0].biomes = Some(container(biome_palette(), Some(vec![0])));
    chunk
}

/// The packed chunk plus the two stored entity shapes of the template: a
/// `DUMMY` entry over a sculk sensor (cell 1030 of the bottom section) and a
/// mob spawner carrying every key the recording shows.
pub(crate) fn block_entity_chunk() -> Chunk {
    let mut chunk = packed_chunk();
    chunk.sections[0].block_states = Some(container(block_palette(), bottom_data(true)));
    if let NbtTag::Compound(root) = &mut chunk.root {
        root.insert(
            "block_entities".to_owned(),
            NbtTag::List(vec![dummy_entity(), spawner_entity()]),
        );
    }
    chunk
}

/// Twenty-four single-value sections: air blocks, plains biomes, no light.
fn sections() -> Vec<Section> {
    (-4_i8..=19)
        .map(|y| Section {
            y,
            block_states: Some(container(vec![string("minecraft:air")], None)),
            biomes: Some(container(vec![string("minecraft:plains")], None)),
            block_light: None,
            sky_light: None,
        })
        .collect()
}

/// The stored `Heightmaps` compound: the words are fixture values, their
/// count differs per map exactly like the stored template's (long maps and
/// short ones side by side).
fn heightmaps() -> NbtTag {
    NbtTag::Compound(BTreeMap::from([
        (
            "MOTION_BLOCKING".to_owned(),
            NbtTag::LongArray(vec![11, 22]),
        ),
        (
            "MOTION_BLOCKING_NO_LEAVES".to_owned(),
            NbtTag::LongArray(vec![33]),
        ),
        ("WORLD_SURFACE".to_owned(), NbtTag::LongArray(vec![44])),
    ]))
}

/// The packed cells of the bottom section: stone at cell 5, water at cell
/// 4000, and (when asked for) a sculk sensor at cell 1030 — block y −60,
/// local x 6, local z 0, the coordinate of the `DUMMY` entity below.
fn bottom_data(with_sculk: bool) -> Option<Vec<i64>> {
    let mut words = vec![0_i64; 256];
    poke(&mut words, 5, 1);
    poke(&mut words, 4000, 2);
    if with_sculk {
        poke(&mut words, 1030, 3);
    }
    Some(words)
}

/// Sets one 4-bit cell of the packed section (16 cells per long).
fn poke(words: &mut [i64], cell: usize, value: i64) {
    words[cell / 16] |= value << ((cell % 16) * 4);
}

/// The block palette of the bottom section: three entries, four with the
/// sculk sensor the `DUMMY` entry above resolves its type through.
fn block_palette() -> Vec<NbtTag> {
    let mut palette = vec![
        string("minecraft:air"),
        string("minecraft:stone"),
        string("minecraft:water"),
    ];
    palette.push(string("minecraft:sculk_sensor"));
    palette
}

/// The biome palette of the bottom section.
fn biome_palette() -> Vec<NbtTag> {
    vec![string("minecraft:plains"), string("minecraft:forest")]
}

/// A `DUMMY` entity at world (102, −60, −32) = local (6, 0) of (6, −2).
fn dummy_entity() -> NbtTag {
    NbtTag::Compound(BTreeMap::from([
        ("id".to_owned(), string("DUMMY")),
        ("x".to_owned(), NbtTag::Int(102)),
        ("y".to_owned(), NbtTag::Int(-60)),
        ("z".to_owned(), NbtTag::Int(-32)),
    ]))
}

/// A mob spawner at world (97, −60, −32) = local (1, 0): the four record
/// keys, the seven spawner settings as stored (a `BTreeMap` keeps them
/// alphabetical — the wire order is the recorder's, see `wire_block_entity`)
/// and the three keys the wire drops.
fn spawner_entity() -> NbtTag {
    NbtTag::Compound(BTreeMap::from([
        ("Delay".to_owned(), NbtTag::Short(20)),
        ("MaxNearbyEntities".to_owned(), NbtTag::Short(8)),
        ("MaxSpawnDelay".to_owned(), NbtTag::Short(800)),
        ("MinSpawnDelay".to_owned(), NbtTag::Short(200)),
        ("RequiredPlayerRange".to_owned(), NbtTag::Short(16)),
        ("SpawnCount".to_owned(), NbtTag::Short(4)),
        ("SpawnRange".to_owned(), NbtTag::Short(4)),
        (
            "SpawnData".to_owned(),
            NbtTag::Compound(BTreeMap::from([(
                "id".to_owned(),
                string("minecraft:zombie"),
            )])),
        ),
        (
            "SpawnPotentials".to_owned(),
            NbtTag::Compound(BTreeMap::new()),
        ),
        ("components".to_owned(), NbtTag::Compound(BTreeMap::new())),
        ("id".to_owned(), string("minecraft:mob_spawner")),
        ("x".to_owned(), NbtTag::Int(97)),
        ("y".to_owned(), NbtTag::Int(-60)),
        ("z".to_owned(), NbtTag::Int(-32)),
    ]))
}

/// One `palette` (+ optional packed `data`) compound of a section.
fn container(palette: Vec<NbtTag>, data: Option<Vec<i64>>) -> NbtTag {
    let mut compound = BTreeMap::new();
    compound.insert("palette".to_owned(), NbtTag::List(palette));
    if let Some(words) = data {
        compound.insert("data".to_owned(), NbtTag::LongArray(words));
    }
    NbtTag::Compound(compound)
}

/// One string tag.
fn string(value: &str) -> NbtTag {
    NbtTag::String(value.to_owned())
}
