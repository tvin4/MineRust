//! Block entity records of the chunk payload: one record per stored entry.
//!
//! SOURCE: parity trace of vanilla 26.3 (`parity/block_entities`) — a record
//! is `[u8 packed xz = (x & 15) << 4 | (z & 15)][i16 y][VarInt type
//! id][network NBT]`, and the recorded nine payloads pin the rules this file
//! follows:
//!
//! * the record count and coordinates equal the stored `block_entities` list,
//!   and every recorded type id equals either the stored entry's `id` or —
//!   for entries the template stores as `DUMMY` — the `block_entity_type`
//!   of the block under the coordinate (checked against all recorded
//!   records; the single deviation, chunk (7, −2), has vanilla records but no
//!   stored entries at all, and is reported by the parity test of 3.3f);
//! * the wire drops the keys `id`, `x`, `y`, `z`, `components`,
//!   `keepPacked`, `cursors`, `listener`, `last_vibration_frequency` and
//!   `SpawnPotentials` — every recorded non-spawner record is exactly one
//!   `00` byte, the recorded mob spawner is exactly the eight keys below;
//! * a mob spawner's children go in the recorded order `SPAWNER_ORDER`,
//!   which is not the alphabetical order a decoded `BTreeMap` hands over;
//! * record order is the stored order: vanilla's own order matches the
//!   stored list only in part of the chunks and is reported by the 3.3f
//!   parity test, never guessed at here.

use std::collections::BTreeMap;

use protocol::{BlockEntity, NbtTag};

use crate::chunk::Chunk;
use crate::wire::{WireError, WireRefs};
use crate::wire_section::Prepared;

/// Stored keys the recorded wire never carries (see the module doc).
const OMITTED: [&str; 10] = [
    "id",
    "x",
    "y",
    "z",
    "components",
    "keepPacked",
    "cursors",
    "listener",
    "last_vibration_frequency",
    "SpawnPotentials",
];

/// Order of a mob spawner's children as the recording sends them
/// (`parity/block_entities` on the spawner of chunk (5, −1), 175 bytes).
const SPAWNER_ORDER: [&str; 8] = [
    "MaxNearbyEntities",
    "RequiredPlayerRange",
    "SpawnCount",
    "SpawnData",
    "MaxSpawnDelay",
    "SpawnRange",
    "Delay",
    "MinSpawnDelay",
];

/// The stored id of an entry whose type the template did not resolve.
const DUMMY: &str = "DUMMY";

/// The type whose children carry the recorded wire order.
const SPAWNER: &str = "minecraft:mob_spawner";

/// The block entity records of a chunk, in the order of the stored list.
pub(crate) fn records(
    chunk: &Chunk,
    sections: &[Prepared],
    refs: &dyn WireRefs,
) -> Result<Vec<BlockEntity>, WireError> {
    entity_list(chunk)?
        .iter()
        .map(|item| record(chunk, item, sections, refs))
        .collect()
}

/// Reads the stored `block_entities` list of the chunk root.
fn entity_list(chunk: &Chunk) -> Result<&Vec<NbtTag>, WireError> {
    match &chunk.root {
        NbtTag::Compound(root) => match root.get("block_entities") {
            Some(NbtTag::List(items)) => Ok(items),
            _ => Err(chunk.wrong_type("block_entities")),
        },
        _ => Err(chunk.wrong_type("<root>")),
    }
}

/// Builds one record from one stored entry.
fn record(
    chunk: &Chunk,
    item: &NbtTag,
    sections: &[Prepared],
    refs: &dyn WireRefs,
) -> Result<BlockEntity, WireError> {
    let entry = match item {
        NbtTag::Compound(entry) => entry,
        _ => return Err(chunk.wrong_type("block_entities[].<not a compound>")),
    };
    let (local_x, local_z, y) = coordinates(chunk, entry)?;
    let stored = string(chunk, entry, "id")?;
    let name: &str = if stored == DUMMY {
        block_under(chunk, sections, local_x, local_z, i32::from(y))?
    } else {
        &stored
    };
    let type_id = refs
        .block_entity_type_id(name)
        .ok_or_else(|| WireError::UnknownBlockEntity {
            x: chunk.x,
            z: chunk.z,
            name: name.to_owned(),
        })?;
    Ok(BlockEntity {
        local_x,
        local_z,
        y,
        type_id,
        nbt: nbt_bytes(entry, &stored)?,
    })
}

/// Local coordinates of the record: the entity must sit inside this chunk
/// and its y must fit the record's `i16`.
fn coordinates(
    chunk: &Chunk,
    entry: &BTreeMap<String, NbtTag>,
) -> Result<(u8, u8, i16), WireError> {
    let (world_x, world_y, world_z) = (
        int(chunk, entry, "x")?,
        int(chunk, entry, "y")?,
        int(chunk, entry, "z")?,
    );
    let outside = || WireError::EntityPosition {
        chunk_x: chunk.x,
        chunk_z: chunk.z,
        x: world_x,
        y: world_y,
        z: world_z,
    };
    let local_x = world_x - chunk.x * 16;
    let local_z = world_z - chunk.z * 16;
    if !(0..16).contains(&local_x) || !(0..16).contains(&local_z) {
        return Err(outside());
    }
    let y = i16::try_from(world_y).map_err(|_| outside())?;
    Ok((local_x as u8, local_z as u8, y))
}

/// The block name under a coordinate: the lookup a `DUMMY` entry is
/// resolved through (the template's stand-in for an unresolved type).
fn block_under<'a>(
    chunk: &Chunk,
    sections: &'a [Prepared],
    local_x: u8,
    local_z: u8,
    y: i32,
) -> Result<&'a str, WireError> {
    let wanted = y >> 4;
    let section = sections
        .iter()
        .find(|section| i32::from(section.y) == wanted)
        .ok_or(WireError::MissingSection {
            x: chunk.x,
            z: chunk.z,
            y: wanted.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8,
        })?;
    // Cell order inside a section: x fastest, then z, then y in the section.
    let cell = local_x as usize + local_z as usize * 16 + (y & 15) as usize * 256;
    Ok(section.names[section.cells[cell]].as_str())
}

/// The record's network NBT: every stored key the recording sends, in the
/// order the recording lists them. No key left means one `00` byte.
fn nbt_bytes(entry: &BTreeMap<String, NbtTag>, id: &str) -> Result<Vec<u8>, WireError> {
    let mut children: Vec<(&String, &NbtTag)> = entry
        .iter()
        .filter(|(key, _)| !OMITTED.contains(&key.as_str()))
        .collect();
    if id == SPAWNER {
        children.sort_by_key(|(key, _)| {
            SPAWNER_ORDER
                .iter()
                .position(|ordered| *ordered == key.as_str())
                .unwrap_or(usize::MAX)
        });
    }
    if children.is_empty() {
        return Ok(vec![0x00]);
    }
    let mut out = vec![0x0A];
    for (key, value) in children {
        value.encode_entry(key, &mut out)?;
    }
    out.push(0x00);
    Ok(out)
}

/// An `int` child of a stored entry.
fn int(chunk: &Chunk, entry: &BTreeMap<String, NbtTag>, field: &str) -> Result<i32, WireError> {
    match entry.get(field) {
        Some(NbtTag::Int(value)) => Ok(*value),
        _ => Err(chunk.wrong_type(&format!("block_entities[].{field}"))),
    }
}

/// A `string` child of a stored entry.
fn string(
    chunk: &Chunk,
    entry: &BTreeMap<String, NbtTag>,
    field: &str,
) -> Result<String, WireError> {
    match entry.get(field) {
        Some(NbtTag::String(value)) => Ok(value.clone()),
        _ => Err(chunk.wrong_type(&format!("block_entities[].{field}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire_fixture::{self, TestRefs};
    use crate::wire_section;

    /// The fixture converted: both records built from the stored list.
    fn converted() -> Vec<BlockEntity> {
        let chunk = wire_fixture::block_entity_chunk();
        let sections = wire_section::prepare(&chunk, &TestRefs).expect("fixture prepares");
        records(&chunk, &sections, &TestRefs).expect("fixture converts")
    }

    #[test]
    fn records_keep_the_stored_order_and_get_local_coordinates() {
        let records = converted();
        assert_eq!(records.len(), 2, "one DUMMY entry and one spawner");
        // First stored entry: the DUMMY over a sculk sensor at local (6, 0).
        assert_eq!(
            (records[0].local_x, records[0].local_z, records[0].y),
            (6, 0, -60)
        );
        assert_eq!(
            records[0].type_id, 34,
            "DUMMY resolves through the block under the coordinate"
        );
        assert_eq!(records[0].nbt, vec![0x00], "its keys are the dropped four");
        // Second stored entry: the mob spawner at local (1, 0).
        assert_eq!(
            (records[1].local_x, records[1].local_z, records[1].y),
            (1, 0, -60)
        );
        assert_eq!(records[1].type_id, 9, "stored id resolved by the report");
    }

    #[test]
    fn dropped_keys_never_reach_the_wire_and_the_spawner_keeps_its_order() {
        let records = converted();
        let nbt = records[1].nbt.as_slice();
        assert_eq!(nbt.first(), Some(&0x0A), "compound root without a name");
        assert_eq!(nbt.last(), Some(&0x00), "compound terminator");
        for dropped in ["SpawnPotentials", "components", "keepPacked", "cursors"] {
            assert!(
                key_position(nbt, dropped).is_none(),
                "{dropped} was dropped by the recording, not sent"
            );
        }
        let written: Vec<usize> = SPAWNER_ORDER
            .iter()
            .map(|key| key_position(nbt, key).unwrap_or(usize::MAX))
            .collect();
        assert!(
            written.iter().all(|position| *position != usize::MAX),
            "all eight recorded keys are present: {written:?}"
        );
        let mut sorted = written.clone();
        sorted.sort_unstable();
        assert_eq!(
            written, sorted,
            "the children go in the recorded, non-alphabetical order"
        );
        // The whole NBT decodes to one compound with exactly those keys.
        let (tag, used) = NbtTag::decode_network(nbt).expect("record NBT decodes");
        assert_eq!(used, nbt.len(), "the bytes are one complete value");
        let NbtTag::Compound(children) = tag else {
            panic!("the record is a compound, got {tag:?}");
        };
        assert_eq!(children.len(), SPAWNER_ORDER.len(), "eight keys survive");
        assert_eq!(
            children.get("Delay"),
            Some(&NbtTag::Short(20)),
            "values pass through untouched"
        );
    }

    #[test]
    fn an_entity_outside_the_chunk_is_named() {
        let mut chunk = wire_fixture::block_entity_chunk();
        let entry = first_entity(&mut chunk);
        entry.insert("x".to_owned(), NbtTag::Int(6 * 16 + 16));
        let sections = wire_section::prepare(&chunk, &TestRefs).expect("fixture prepares");
        let error = records(&chunk, &sections, &TestRefs).expect_err("outside");
        assert!(
            matches!(error, WireError::EntityPosition { x: 112, .. }),
            "{error}"
        );
    }

    #[test]
    fn an_unknown_type_names_the_stored_name() {
        let mut chunk = wire_fixture::block_entity_chunk();
        let entry = first_entity(&mut chunk);
        entry.insert(
            "id".to_owned(),
            NbtTag::String("minecraft:no_type".to_owned()),
        );
        let sections = wire_section::prepare(&chunk, &TestRefs).expect("fixture prepares");
        let error = records(&chunk, &sections, &TestRefs).expect_err("unknown");
        assert!(
            matches!(
                &error,
                WireError::UnknownBlockEntity { name, .. } if name == "minecraft:no_type"
            ),
            "{error}"
        );
    }

    /// The first stored entity of the fixture, for a test to rewrite.
    fn first_entity(chunk: &mut Chunk) -> &mut BTreeMap<String, NbtTag> {
        match &mut chunk.root {
            NbtTag::Compound(root) => match root.get_mut("block_entities") {
                Some(NbtTag::List(items)) => match &mut items[0] {
                    NbtTag::Compound(entry) => entry,
                    other => panic!("the fixture stores a compound, got {other:?}"),
                },
                other => panic!("the fixture stores a list, got {other:?}"),
            },
            other => panic!("the fixture stores a compound root, got {other:?}"),
        }
    }

    /// Offset of a length-prefixed child name inside a record's NBT.
    fn key_position(nbt: &[u8], name: &str) -> Option<usize> {
        let length = u16::try_from(name.len()).ok()?;
        let mut needle = length.to_be_bytes().to_vec();
        needle.extend_from_slice(name.as_bytes());
        nbt.windows(needle.len())
            .position(|window| window == needle)
    }
}
