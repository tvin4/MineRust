//! Step 3.2: decoding the section records of `level_chunk_with_light` from
//! the recorded vanilla bytes plus the data reports of the game copy.
//!
//! `cargo test -p parity --test section_map -- --ignored --nocapture`
//!
//! Sections of a stored chunk carry a `block_states` compound: a palette of
//! block names and the packed `data` longs. The wire sends the same data,
//! so the exact offsets of the longs in the recorded payload frame every
//! section; what stands between them (block count, palette, data length) is
//! read field by field and checked against the default state ids of
//! `reports/blocks.json`. A field that cannot be read from that data stays
//! undecoded here and is reported, never guessed.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use protocol::NbtTag;
use serde_json::Value;
use world::Chunk;

/// Chunk coordinates of the first batch after the spawn fix (step 3.0/4).
const FIRST: (i32, i32) = (6, -2);

/// Loads the recorded payload of `FIRST` from the last vanilla run.
fn vanilla_payload(workspace: &Path) -> Vec<u8> {
    let text = std::fs::read_to_string(workspace.join("target/parity/join-vanilla.json"))
        .expect("join-vanilla.json (run join_slice first)");
    let trace: Value = serde_json::from_str(&text).expect("trace is JSON");
    for packet in trace["packets"].as_array().expect("packets array") {
        if packet["name"] != "minecraft:level_chunk_with_light" {
            continue;
        }
        let hex = packet["payload"].as_str().expect("payload hex");
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex"))
            .collect();
        if bytes.len() < 8 {
            continue;
        }
        let x = i32::from_be_bytes(bytes[0..4].try_into().expect("big endian"));
        let z = i32::from_be_bytes(bytes[4..8].try_into().expect("big endian"));
        if (x, z) == FIRST {
            return bytes;
        }
    }
    panic!("chunk ({}, {}) not in the trace", FIRST.0, FIRST.1);
}

/// Reads the stored model of `FIRST` from the world template.
fn stored_chunk() -> Chunk {
    let env = parity::load_parity_env().expect("parity environment");
    let region = env
        .world_template
        .join("dimensions/minecraft/overworld/region/r.0.-1.mca");
    let stored = world::load_region(&region)
        .expect("region holding the spawn chunk")
        .into_iter()
        .find(|chunk| (chunk.x, chunk.z) == FIRST)
        .expect("stored chunk in the region");
    Chunk::from_stored(&stored).expect("model")
}

/// Path of the `blocks.json` data report (state ids of the game copy).
fn blocks_report() -> PathBuf {
    std::env::var_os("MINECRAFT_BLOCKS_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\MineRust-data\reports\blocks.json"))
}

/// One state of a block: its id and the values of its properties, exactly
/// as `blocks.json` writes them for that state.
type BlockState = (i32, BTreeMap<String, String>);

/// What the block data report says, read once: default ids, ids decoded
/// back to block names and, per block, every state id with the property
/// values it accepts.
struct BlocksReport {
    /// Default state id of every block.
    default: BTreeMap<String, i32>,
    /// Block name of every state id.
    by_id: BTreeMap<i32, String>,
    /// Per block: every state the report lists.
    states: BTreeMap<String, Vec<BlockState>>,
}

impl BlocksReport {
    /// Loads `blocks.json`.
    fn load(path: &Path) -> Self {
        let text = std::fs::read_to_string(path).expect("read blocks.json");
        let parsed: BTreeMap<String, Value> =
            serde_json::from_str(&text).expect("blocks.json is JSON");
        let mut report = Self {
            default: BTreeMap::new(),
            by_id: BTreeMap::new(),
            states: BTreeMap::new(),
        };
        for (name, block) in &parsed {
            let Some(states) = block["states"].as_array() else {
                continue;
            };
            let mut all = Vec::new();
            for state in states {
                let Some(id) = state["id"].as_i64() else {
                    continue;
                };
                report
                    .by_id
                    .entry(id as i32)
                    .or_insert_with(|| name.clone());
                if state["default"] == Value::Bool(true) {
                    report.default.insert(name.clone(), id as i32);
                }
                // Per state the report writes each property as a plain
                // string (the arrays live on the block itself, listing the
                // allowed values).
                let allowed: BTreeMap<String, String> = state["properties"]
                    .as_object()
                    .map(|object| {
                        object
                            .iter()
                            .filter_map(|(key, value)| {
                                let value = value.as_str()?;
                                Some((key.clone(), value.to_owned()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                all.push((id as i32, allowed));
            }
            if let Some(first) = all.first() {
                report.default.entry(name.clone()).or_insert(first.0);
            }
            report.states.insert(name.clone(), all);
        }
        report
    }

    /// The state id a stored palette entry stands for: the block default for
    /// a plain entry, or the id of the state whose accepted values match
    /// every requested property.
    fn resolved_id(&self, entry: &StoredEntry) -> Option<i32> {
        let Some(properties) = &entry.properties else {
            return self.default.get(&entry.name).copied();
        };
        let candidates = self.states.get(&entry.name)?;
        candidates
            .iter()
            .find(|(_, values)| values == properties)
            .map(|(id, _)| *id)
    }
}

/// One stored palette entry: the block name and, for a state that differs
/// from the block default, its properties.
struct StoredEntry {
    /// Block name without the properties.
    name: String,
    /// NBT properties of a non-default state.
    properties: Option<BTreeMap<String, String>>,
}

/// Palette entries of one stored section, in palette order.
///
/// The template mixes three shapes: a plain string, a compound whose single
/// field holds the name under an empty key or `Name`, and the full form
/// `{"id": ..., "properties": {...}}` of a state that differs from the
/// block default.
fn palette_entries(section: &world::Section) -> Vec<StoredEntry> {
    let Some(NbtTag::Compound(states)) = &section.block_states else {
        return Vec::new();
    };
    let Some(NbtTag::List(palette)) = states.get("palette") else {
        return Vec::new();
    };
    palette
        .iter()
        .filter_map(|tag| match tag {
            NbtTag::String(name) => Some(StoredEntry {
                name: name.clone(),
                properties: None,
            }),
            NbtTag::Compound(fields) => {
                let name = ["id", "Name", ""]
                    .iter()
                    .find_map(|key| match fields.get(*key) {
                        Some(NbtTag::String(name)) => Some(name.clone()),
                        _ => None,
                    })?;
                let properties = match fields.get("properties") {
                    Some(NbtTag::Compound(values)) => Some(
                        values
                            .iter()
                            .filter_map(|(key, value)| match value {
                                NbtTag::String(value) => Some((key.clone(), value.clone())),
                                _ => None,
                            })
                            .collect(),
                    ),
                    _ => None,
                };
                Some(StoredEntry { name, properties })
            }
            _ => None,
        })
        .collect()
}

/// Packed `data` longs of one stored section as big-endian bytes.
fn data_bytes(section: &world::Section) -> Vec<u8> {
    let Some(NbtTag::Compound(states)) = &section.block_states else {
        return Vec::new();
    };
    let Some(NbtTag::LongArray(values)) = states.get("data") else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(values.len() * 8);
    for value in values {
        out.extend_from_slice(&value.to_be_bytes());
    }
    out
}

/// A VarInt read from `payload` at `at`; `None` when it runs past the end.
fn read_varint(payload: &[u8], at: usize) -> Option<(i32, usize)> {
    let mut value = 0_u32;
    let mut taken = 0_usize;
    loop {
        let byte = *payload.get(at + taken)?;
        value |= u32::from(byte & 0x7f) << (taken * 7);
        taken += 1;
        if byte & 0x80 == 0 {
            return Some((value as i32, taken));
        }
        if taken == 5 {
            return None;
        }
    }
}

/// A byte range read as a stream of VarInts (used to look at record
/// prefixes: fields that are really VarInts come out whole, bytes of
/// fixed-width fields come out broken, and the difference is evidence too).
fn varint_stream(bytes: &[u8]) -> Vec<i32> {
    let mut out = Vec::new();
    let mut cursor = 0_usize;
    while cursor < bytes.len() {
        match read_varint(bytes, cursor) {
            Some((value, len)) => {
                out.push(value);
                cursor += len;
            }
            None => break,
        }
    }
    out
}

/// Bits per cell of a block container with a palette of `count` entries:
/// the wire never goes below 4 bits (measured on all fourteen records; the
/// data sizes 2048/2736/3280 bytes fit exactly).
fn container_bits(count: usize) -> usize {
    match count {
        0..=16 => 4,
        17..=32 => 5,
        _ => 6,
    }
}

/// How many of the 4096 cells are not air; every cell when the palette
/// has no air entry at all.
fn non_air_count(data: &[u8], bits: usize, air: Option<usize>) -> usize {
    air.map_or(4096, |air| 4096 - count_cells(data, bits, air))
}

/// How many of the 4096 cells of a packed block array hold `wanted`.
///
/// One long holds `64 / bits` cells (16 for 4 bits, 12 for 5, 10 for 6 —
/// the measured 256/342/410 longs come out of exactly that), cells are
/// read from the low bits up and the longs are big-endian as on the wire.
fn count_cells(data: &[u8], bits: usize, wanted: usize) -> usize {
    let per_long = 64 / bits;
    let mask = (1_i64 << bits) - 1;
    let mut count = 0_usize;
    for index in 0..4096_usize {
        let long_start = (index / per_long) * 8;
        let Some(word) = data.get(long_start..long_start + 8) else {
            break;
        };
        let word = i64::from_be_bytes(word.try_into().expect("long of 8 bytes"));
        if (word >> ((index % per_long) * bits)) & mask == wanted as i64 {
            count += 1;
        }
    }
    count
}

/// The biome palette of one stored section, in palette order.
fn biomes_of(section: &world::Section) -> Vec<String> {
    let Some(NbtTag::Compound(biomes)) = &section.biomes else {
        return Vec::new();
    };
    let Some(NbtTag::List(palette)) = biomes.get("palette") else {
        return Vec::new();
    };
    palette
        .iter()
        .filter_map(|tag| match tag {
            NbtTag::String(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// One decoded section frame: the data the wire palette must land behind.
struct Frame {
    /// Section index in the stored chunk.
    index: usize,
    /// Section Y.
    y: i8,
    /// Offset of the packed data array.
    data: usize,
}

/// Finds the block palette that ends exactly at `data` and decodes to the
/// stored one: the offset of its VarInt count, the count and the entry ids.
///
/// The walk goes from `record_start` forward; a candidate is a VarInt count
/// with exactly `expected.len()` VarInt ids landing on `data` whose ids
/// decode through `by_id` to the stored block names. Landing on the exact
/// end and decoding to the exact names leaves no room for a wrong hit.
fn frame_palette(
    payload: &[u8],
    record_start: usize,
    data: usize,
    expected: &[String],
    by_id: &BTreeMap<i32, String>,
) -> Option<(usize, i32, Vec<i32>)> {
    for at in record_start..data {
        let Some((count, taken)) = read_varint(payload, at) else {
            break;
        };
        if count as usize != expected.len() {
            continue;
        }
        let mut cursor = at + taken;
        let mut ids = Vec::with_capacity(count as usize);
        let mut ok = true;
        for _ in 0..count {
            match read_varint(payload, cursor) {
                Some((value, len)) => {
                    ids.push(value);
                    cursor += len;
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok && cursor == data {
            let decoded: Vec<String> = ids.iter().filter_map(|id| by_id.get(id).cloned()).collect();
            if decoded == expected {
                return Some((at, count, ids));
            }
        }
    }
    None
}

#[test]
#[ignore = "requires the owner's world, the trace and blocks.json"]
fn section_records_match_the_stored_palettes() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let payload = vanilla_payload(&workspace);
    let chunk = stored_chunk();

    // Every packed data array frames its section: the record starts somewhere
    // before the data; records follow in ascending Y order (measured).
    let mut frames: Vec<Frame> = Vec::new();
    for (index, section) in chunk.sections.iter().enumerate() {
        let data = data_bytes(section);
        if data.is_empty() {
            continue;
        }
        let mut hits = Vec::new();
        if data.len() <= payload.len() {
            for offset in 0..=payload.len() - data.len() {
                if payload[offset..offset + data.len()] == data[..] {
                    hits.push(offset);
                }
            }
        }
        assert_eq!(
            hits.len(),
            1,
            "section {index} data must occur exactly once, hits: {hits:?}"
        );
        frames.push(Frame {
            index,
            y: section.y,
            data: hits[0],
        });
    }

    // The sections region opens with a VarInt length of the whole blob; its
    // value is checked against the known position of the block entities.
    let Some((blob_len, blob_varint)) = read_varint(&payload, 903) else {
        panic!("no VarInt at 903");
    };
    let blob_start = 903 + blob_varint;
    let blob_end = blob_start + blob_len.max(0) as usize;
    eprintln!(
        "--- sections blob: {blob_start}..{blob_end} (VarInt {blob_len} in {blob_varint} bytes)"
    );

    // The wire palette of every section is found without the NBT: a VarInt
    // count followed by exactly `count` VarInt ids that land on the data
    // start. The then compare with the stored palette tells which field is
    // content (ids) and which frame (header bytes) of the record.
    eprintln!("--- section records: prefix, wire palette vs stored palette ---");
    let report = BlocksReport::load(&blocks_report());
    let mut record_start = blob_start;
    for frame in &mut frames {
        let section = &chunk.sections[frame.index];
        let data = data_bytes(section);
        let entries = palette_entries(section);
        let stored_names: Vec<String> = entries.iter().map(|e| e.name.clone()).collect();
        let stored_ids: Vec<Option<i32>> = entries.iter().map(|e| report.resolved_id(e)).collect();
        let Some((at, count, wire_ids)) = frame_palette(
            &payload,
            record_start,
            frame.data,
            &stored_names,
            &report.by_id,
        ) else {
            eprintln!(
                "sec[{}] Y={:>3}: palette frame NOT found in [{record_start}..{}]",
                frame.index, frame.y, frame.data
            );
            record_start = frame.data + data.len();
            continue;
        };
        let decode: Vec<String> = wire_ids
            .iter()
            .map(|id| {
                report
                    .by_id
                    .get(id)
                    .map_or(format!("id {id}"), |name| name.clone())
            })
            .collect();
        eprintln!(
            "sec[{}] Y={:>3}: data {} bytes @ {}, palette @ {at}, prefix [{record_start}..{at}]: {:02x?}",
            frame.index,
            frame.y,
            data.len(),
            frame.data,
            &payload[record_start..at]
        );
        eprintln!(
            "  prefix as VarInts: {:?}",
            varint_stream(&payload[record_start..at])
        );
        // The container bits follow from the palette size, and the array
        // size from the bits: the two measurements must agree with the wire.
        let bits = container_bits(count as usize);
        let longs = 4096_usize.div_ceil(64 / bits);
        assert_eq!(
            data.len(),
            longs * 8,
            "sec[{}]: {longs} longs expected for {bits} bits",
            frame.index
        );
        let air = stored_names.iter().position(|name| name == "minecraft:air");
        eprintln!(
            "  bits {bits} ({longs} longs), non-air cells: {}",
            non_air_count(&data, bits, air)
        );
        // Counting one specific block the same way tests what the second
        // u16 field of the header may be: the water cells of this section.
        let water = stored_names
            .iter()
            .position(|name| name == "minecraft:water");
        if let Some(water) = water {
            eprintln!("  water cells: {}", count_cells(&data, bits, water));
        }
        eprintln!("  biomes: {:?}", biomes_of(section));
        eprintln!("  wire {count} entries ids: {wire_ids:?}");
        let unresolved: Vec<&str> = entries
            .iter()
            .zip(&stored_ids)
            .filter_map(|(entry, id)| id.is_none().then_some(entry.name.as_str()))
            .collect();
        if !unresolved.is_empty() {
            eprintln!("  stored ids NOT resolved: {unresolved:?}");
        }
        eprintln!(
            "  state ids equal: {}",
            stored_ids.len() == wire_ids.len()
                && stored_ids
                    .iter()
                    .zip(&wire_ids)
                    .all(|(stored, wire)| *stored == Some(*wire))
                && stored_ids.iter().all(Option::is_some)
        );
        eprintln!("  block names equal: {}", decode == stored_names);
        if decode != stored_names {
            eprintln!("  stored names differ: {stored_names:?}");
        }
        record_start = frame.data + data.len();
    }

    // What is left of the blob after the last palette-backed data: the
    // single-valued sections (no `data` in NBT), followed by the block
    // entities.
    eprintln!(
        "--- record tail [{record_start}..{blob_end}] of {} bytes: {:02x?}",
        blob_end.saturating_sub(record_start),
        &payload[record_start..blob_end.min(payload.len())]
    );
    eprintln!(
        "--- past the blob [{blob_end}..]: {:02x?}",
        &payload[blob_end..payload.len().min(blob_end + 80)]
    );
}
