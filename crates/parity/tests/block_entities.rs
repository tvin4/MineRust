//! Step 3.2/4 (owner check 2): the block entity records inside a chunk
//! payload against the stored chunk and `registries.json`.
//!
//! `cargo test -p parity --test block_entities -- --ignored --nocapture`
//!
//! The hypothesis under test (owner, not yet accepted): after the sections
//! blob a block entity is
//! `[u8 packed xz = (x & 15) << 4 | (z & 15)][i16 BE y][VarInt type id]
//! [NBT, 0 = empty compound]`. The wire ids are matched against
//! `minecraft:block_entity_type` of `reports/registries.json`, the
//! coordinates against `block_entities` of the stored chunk, and the
//! blocks at those coordinates against the stored palettes.

#[path = "common/chunk_trace.rs"]
mod chunk_trace;

use std::collections::BTreeMap;
use std::path::Path;

use chunk_trace::{be_u16, read_varint, vanilla_payload};
use protocol::NbtTag;
use serde_json::Value;

/// Byte holding the VarInt length of the sections blob.
const BLOB_LEN_AT: usize = 903;

/// One block entity record as read off the wire.
struct WireEntity {
    /// Local block x inside the chunk (0..16).
    local_x: i32,
    /// Local block z inside the chunk (0..16).
    local_z: i32,
    /// Block y.
    y: i32,
    /// VarInt type id.
    type_id: i32,
    /// Byte standing where the entity NBT should start.
    nbt: u8,
}

/// Reads the block entity group: its VarInt count and the records after it.
fn read_wire_entities(payload: &[u8], at: usize) -> (Vec<WireEntity>, usize) {
    let (count, taken) = read_varint(payload, at).expect("entity count VarInt");
    let mut cursor = at + taken;
    let mut out = Vec::new();
    for _ in 0..count {
        let byte = *(payload.get(cursor).expect("packed xz"));
        let y = be_u16(payload, cursor + 1).expect("y i16") as i16;
        cursor += 3;
        let (type_id, len) = read_varint(payload, cursor).expect("type id VarInt");
        cursor += len;
        let nbt = *(payload.get(cursor).expect("NBT tag byte"));
        cursor += 1;
        out.push(WireEntity {
            local_x: i32::from(byte >> 4),
            local_z: i32::from(byte & 0x0f),
            y: i32::from(y),
            type_id,
            nbt,
        });
    }
    (out, cursor)
}

/// The `block_entities` list of the stored chunk, untouched.
fn stored_entities(chunk: &world::Chunk) -> Vec<BTreeMap<String, NbtTag>> {
    let NbtTag::Compound(root) = &chunk.root else {
        return Vec::new();
    };
    match root.get("block_entities") {
        Some(NbtTag::List(items)) => items
            .iter()
            .filter_map(|item| match item {
                NbtTag::Compound(map) => Some(map.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// `protocol_id` of every `minecraft:block_entity_type` name.
fn block_entity_ids(path: &Path) -> BTreeMap<i32, String> {
    let text = std::fs::read_to_string(path).expect("read registries.json");
    let report: Value = serde_json::from_str(&text).expect("registries.json is JSON");
    report["minecraft:block_entity_type"]["entries"]
        .as_object()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|(name, value)| {
                    let id = value["protocol_id"].as_i64()?;
                    Some((id as i32, name.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[test]
#[ignore = "requires the owner's files and the recorded trace"]
fn ten_block_entities_frame_against_the_stored_chunk() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let env = parity::load_parity_env().expect("parity environment");
    let chunk = chunk_trace::stored_chunk(&env.world_template);
    let payload = vanilla_payload(&workspace);

    let (blob_len, taken) = read_varint(&payload, BLOB_LEN_AT).expect("blob length VarInt");
    let block_entities_at = BLOB_LEN_AT + taken + blob_len as usize;

    let (wire, after) = read_wire_entities(&payload, block_entities_at);
    eprintln!(
        "block entity group @ {block_entities_at}: {} records, ends @ {after}",
        wire.len()
    );

    let stored = stored_entities(&chunk);
    eprintln!("stored block_entities: {}", stored.len());
    eprintln!("group bytes: {:02x?}", &payload[block_entities_at..after]);
    assert_eq!(wire.len(), stored.len(), "wire count vs NBT count");

    let registries = env
        .config
        .registries_report
        .as_deref()
        .map(Path::new)
        .expect("registries_report path");
    let by_id = block_entity_ids(registries);

    let mut matched = 0_usize;
    let mut types_agree = 0_usize;
    for (index, record) in wire.iter().enumerate() {
        let (x, z) = (chunk.x * 16 + record.local_x, chunk.z * 16 + record.local_z);
        let target = stored.iter().find(|entity| {
            int_field(entity, "x") == Some(x)
                && int_field(entity, "y") == Some(record.y)
                && int_field(entity, "z") == Some(z)
        });
        let Some(entity) = target else {
            panic!(
                "wire record {index} at ({x}, {}, {z}) has no NBT twin",
                record.y
            );
        };
        matched += 1;
        let id = str_field(entity, "id").unwrap_or_default();
        let registered = by_id.get(&record.type_id).cloned().unwrap_or_default();
        let agree = registered == id;
        types_agree += usize::from(agree);
        eprintln!(
            "  ({x}, {}, {z}) wire type {} = {registered}, nbt {id}, regimes agree: {agree}",
            record.y, record.type_id,
        );
    }
    eprintln!(
        "--- {matched} coordinates matched, types agree {types_agree}/{}",
        wire.len()
    );
    assert_eq!(matched, wire.len(), "every wire coordinate has an NBT twin");
    assert!(wire.iter().all(|record| record.nbt == 0), "empty NBT only");
    assert_eq!(after, block_entities_at + 1 + wire.len() * 5, "5 B records");
    assert_eq!(
        types_agree,
        wire.len(),
        "wire type id must equal the stored entity id"
    );

    // The block each entity sits on must be the block the entity is named
    // after (`minecraft:sculk_sensor` on a sculk sensor, ...). Which axis
    // runs fastest inside a packed section is not written down in the
    // data, so every axis order is tried and the order that puts all ten
    // entities on their own block is the one the world data uses.
    eprintln!("--- block under each entity, per cell order ---");
    let mut best = (0_usize, Vec::new());
    for order in ORDERS {
        let hits = stored
            .iter()
            .filter(|entity| {
                let (Some(x), Some(y), Some(z)) = (
                    int_field(entity, "x"),
                    int_field(entity, "y"),
                    int_field(entity, "z"),
                ) else {
                    return false;
                };
                let id = str_field(entity, "id").unwrap_or_default();
                block_at(&chunk, x, y, z, order).as_deref() == Some(id.as_str())
            })
            .count();
        eprintln!(
            "  order [fast]={} middle={} slow={}: {hits}/{}",
            axis_name(order[0]),
            axis_name(order[1]),
            axis_name(order[2]),
            stored.len()
        );
        if hits > best.0 {
            best = (hits, order.to_vec());
        }
    }
    assert_eq!(
        best.0,
        stored.len(),
        "one cell order must put every entity on its own block, best {:?}",
        best.1
    );
}

/// Owner step 3: for every recorded chunk — the block entity records (the
/// (7, -2) case in the open: vanilla sends 2, the stored list is empty) and
/// the block data behind them, section by section against the stored NBT.
/// How far the recording's world sits from the served template decides
/// whether the phantom records are a reader bug or a stale recording.
#[test]
#[ignore = "requires the owner's files and the recorded trace"]
fn every_recorded_chunk_against_its_stored_one() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let env = parity::load_parity_env().expect("parity environment");
    let registries = env
        .config
        .registries_report
        .as_deref()
        .map(Path::new)
        .expect("registries_report path");
    let by_id = block_entity_ids(registries);

    for (coord, bytes) in chunk_trace::all_payloads(&workspace) {
        let mut reader = protocol::PacketReader::new(&bytes);
        let decoded = match protocol::LevelChunkWithLight::decode_payload(&mut reader) {
            Ok(chunk) => chunk,
            Err(error) => {
                eprintln!("chunk {coord:?}: payload does not decode: {error}");
                continue;
            }
        };
        let Some(chunk) = chunk_trace::stored_chunk_at(&env.world_template, coord.0, coord.1)
        else {
            eprintln!("chunk {coord:?}: not in the stored world");
            continue;
        };
        let stored = stored_entities(&chunk);
        eprintln!(
            "chunk {coord:?}: vanilla {} records, stored {} entries",
            decoded.block_entities.len(),
            stored.len()
        );
        for entity in &decoded.block_entities {
            report_wire_record(coord, entity, &stored, &by_id, &chunk);
        }
        if stored.len() != decoded.block_entities.len() {
            if let NbtTag::Compound(root) = &chunk.root {
                eprintln!("  stored root keys: {:?}", root.keys().collect::<Vec<_>>());
                match root.get("block_entities") {
                    Some(value) => {
                        eprintln!("  stored `block_entities`: {}", abbreviate(value, 600))
                    }
                    None => eprintln!("  stored chunk has no `block_entities` key"),
                }
            }
        }
        report_block_data(coord, &decoded, &chunk);
    }
}

/// One section whose packed block data differs: section index, the wire
/// bytes, the stored bytes and the byte positions that do not match.
type DifferingData = (usize, Vec<u8>, Vec<u8>, Vec<usize>);

/// How many sections of block data differ between the recording and the
/// stored chunk; for (7, -2) additionally the palettes and the wired bytes
/// of every differing section, cell by cell.
fn report_block_data(
    coord: (i32, i32),
    decoded: &protocol::LevelChunkWithLight,
    chunk: &world::Chunk,
) {
    let mut differing: Vec<DifferingData> = Vec::new();
    for (index, (recorded, stored)) in decoded.sections.iter().zip(&chunk.sections).enumerate() {
        let wire_bytes = match &recorded.blocks {
            protocol::PaletteContainer::Single(_) => Vec::new(),
            protocol::PaletteContainer::Packed { data, .. } => data.clone(),
        };
        let stored_bytes = stored_data_bytes(stored);
        if wire_bytes != stored_bytes {
            let positions = wire_bytes
                .iter()
                .zip(&stored_bytes)
                .enumerate()
                .filter(|(_, (wire, mine))| wire != mine)
                .map(|(position, _)| position)
                .collect::<Vec<_>>();
            differing.push((index, wire_bytes, stored_bytes, positions));
        }
    }
    eprintln!(
        "  chunk {coord:?}: {}/{} sections differ in block data",
        differing.len(),
        chunk.sections.len()
    );
    if coord != (7, -2) {
        return;
    }
    for (index, wire, stored, positions) in &differing {
        let palette = match &decoded.sections[*index].blocks {
            protocol::PaletteContainer::Single(_) => "single".to_owned(),
            protocol::PaletteContainer::Packed { bits, palette, .. } => {
                format!("{bits} bits, {} ids {palette:?}", palette.len())
            }
        };
        let stored_names = chunk_trace::palette_names(&chunk.sections[*index]);
        eprintln!(
            "  section #{index}: wire [{palette}], stored ({} names) {stored_names:?}; \
             data {} vs {} bytes, {} positions {positions:?}",
            stored_names.len(),
            wire.len(),
            stored.len(),
            positions.len()
        );
        for byte in positions.iter().take(4) {
            for half in [0_usize, 1] {
                let cell = *byte * 2 + half; // two 4-bit cells in one byte
                let label = cell_label(cell, wire, stored, *index, chunk);
                eprintln!("    byte {byte} cell {cell}: {label}");
            }
        }
    }
}

/// A debug string capped at `cap` bytes (only on a character boundary).
fn abbreviate(tag: &protocol::NbtTag, cap: usize) -> String {
    let text = format!("{tag:?}");
    if text.len() <= cap {
        return text;
    }
    let mut cut = cap;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}… ({} bytes total)", &text[..cut], text.len())
}

/// One differing cell of (7, -2): the wire palette id against the stored
/// block name at the same coordinates (`cell = x + z·16 + y·256`, section
/// index 1 = Y −3 = world y −48..−33).
fn cell_label(
    cell: usize,
    wire: &[u8],
    stored: &[u8],
    section: usize,
    chunk: &world::Chunk,
) -> String {
    let nibble = wire.get(cell / 2).map(|byte| {
        if cell.is_multiple_of(2) {
            byte >> 4
        } else {
            byte & 0x0f
        }
    });
    let stored_nibble = stored.get(cell / 2).map(|byte| {
        if cell.is_multiple_of(2) {
            byte >> 4
        } else {
            byte & 0x0f
        }
    });
    let (local_x, rest) = (cell % 16, cell / 16);
    let (local_z, local_y) = (rest % 16, rest / 16);
    let world_y = (section as i32 - 4) * 16 + local_y as i32;
    let world_x = chunk.x * 16 + local_x as i32;
    let world_z = chunk.z * 16 + local_z as i32;
    let block = block_at(chunk, world_x, world_y, world_z, [0, 2, 1]).unwrap_or_default();
    format!(
        "wire id nibble {nibble:?} vs stored nibble {stored_nibble:?}; \
         stored block at ({world_x}, {world_y}, {world_z}) = {block}"
    )
}

/// The packed block data of a stored section as wire bytes: NBT longs,
/// each written big-endian, exactly as the payload frame does it.
fn stored_data_bytes(section: &world::Section) -> Vec<u8> {
    let Some(NbtTag::Compound(states)) = &section.block_states else {
        return Vec::new();
    };
    match states.get("data") {
        Some(NbtTag::LongArray(words)) => {
            words.iter().flat_map(|word| word.to_be_bytes()).collect()
        }
        _ => Vec::new(),
    }
}

/// One wire record: the stored NBT twin it must have (or the block under
/// its coordinate, per cell order, when it has none).
fn report_wire_record(
    coord: (i32, i32),
    entity: &protocol::BlockEntity,
    stored: &[BTreeMap<String, NbtTag>],
    by_id: &BTreeMap<i32, String>,
    chunk: &world::Chunk,
) {
    let (x, z) = (
        coord.0 * 16 + i32::from(entity.local_x),
        coord.1 * 16 + i32::from(entity.local_z),
    );
    let y = i32::from(entity.y);
    let registered = by_id
        .get(&entity.type_id)
        .cloned()
        .unwrap_or_else(|| format!("unregistered type {}", entity.type_id));
    let twin = stored.iter().find(|entry| {
        int_field(entry, "x") == Some(x)
            && int_field(entry, "y") == Some(y)
            && int_field(entry, "z") == Some(z)
    });
    match twin {
        Some(entry) => {
            let id = str_field(entry, "id").unwrap_or_default();
            eprintln!(
                "  ({x}, {y}, {z}) wire {registered}, stored {id}, agree: {}",
                registered == id
            );
        }
        None => {
            let blocks = ORDERS
                .iter()
                .map(|order| block_at(chunk, x, y, z, *order).unwrap_or_default())
                .collect::<Vec<_>>();
            eprintln!(
                "  ({x}, {y}, {z}) wire {registered}, NO STORED TWIN; \
                 block by cell order {blocks:?}"
            );
        }
    }
}

/// The six axis orders inside a packed section, as slow-aspect triples
/// (fastest, middle, slowest axis index; 0 = x, 1 = y, 2 = z).
const ORDERS: [[usize; 3]; 6] = [
    [0, 1, 2],
    [0, 2, 1],
    [1, 0, 2],
    [1, 2, 0],
    [2, 0, 1],
    [2, 1, 0],
];

/// One axis as a letter, for readable order reports.
fn axis_name(axis: usize) -> char {
    match axis {
        0 => 'x',
        1 => 'y',
        2 => 'z',
        _ => '?',
    }
}

/// The stored block name at a world coordinate, under one axis order: the
/// `order` triples give the fastest, middle and slowest axis of the cell
/// index (`fast + middle * 16 + slow * 256`).
fn block_at(chunk: &world::Chunk, x: i32, y: i32, z: i32, order: [usize; 3]) -> Option<String> {
    let section = chunk
        .sections
        .iter()
        .find(|section| i32::from(section.y) == y.div_euclid(16))?;
    let local = |axis: usize| match axis {
        0 => x.rem_euclid(16),
        1 => y.rem_euclid(16),
        _ => z.rem_euclid(16),
    };
    let cell = local(order[0]) + local(order[1]) * 16 + local(order[2]) * 256;
    let names = chunk_trace::palette_names(section);
    let NbtTag::Compound(states) = section.block_states.as_ref()? else {
        return None;
    };
    let index = match states.get("data") {
        // A single-valued section: every cell holds palette entry 0.
        None => 0,
        Some(NbtTag::LongArray(data)) if data.is_empty() => 0,
        Some(NbtTag::LongArray(data)) => {
            let bits = bits_for(names.len());
            let per_long = 64 / bits;
            let long = cell as usize / per_long;
            let word = *data.get(long)?;
            let mask = (1_i64 << bits) - 1;
            ((word >> ((cell as usize % per_long) * bits)) & mask) as usize
        }
        Some(_) => return None,
    };
    names.get(index).cloned()
}

/// Bits per cell of a section palette with `count` entries: the measured
/// floor of 4 on the wire (4 for up to 16 entries, 5 for up to 32, 6 for
/// up to 64).
fn bits_for(count: usize) -> usize {
    count.max(1).next_power_of_two().ilog2().max(4) as usize
}

/// An integer child of an entity compound (`Int`, `Short` or `Byte`).
fn int_field(entity: &BTreeMap<String, NbtTag>, key: &str) -> Option<i32> {
    match entity.get(key) {
        Some(NbtTag::Int(value)) => Some(*value),
        Some(NbtTag::Short(value)) => Some(i32::from(*value)),
        Some(NbtTag::Byte(value)) => Some(i32::from(*value)),
        _ => None,
    }
}

/// A string child of an entity compound.
fn str_field(entity: &BTreeMap<String, NbtTag>, key: &str) -> Option<String> {
    match entity.get(key) {
        Some(NbtTag::String(value)) => Some(value.clone()),
        _ => None,
    }
}
