//! Loaders shared by the chunk framing tests: the recorded vanilla trace
//! and the stored chunk of the world template.
//!
//! Included by path (`#[path = "common/chunk_trace.rs"] mod chunk_trace;`)
//! from the test crates that need it: a file in a subdirectory of `tests/`
//! is not compiled as a test target of its own, and every including crate
//! compiles only what it calls.
#![allow(dead_code)]

use std::path::Path;

use serde_json::Value;
use world::{Chunk, StoredChunk};

/// Chunk coordinates of the first batch after the spawn fix (step 3.0/4).
pub const FIRST: (i32, i32) = (6, -2);

/// Hex string of even length to bytes.
pub fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex"))
        .collect()
}

/// Payload bytes of the `level_chunk_with_light` sent for `coords` in the
/// last vanilla run, as recorded by `join_slice`.
pub fn payload_of(workspace: &Path, coords: (i32, i32)) -> Vec<u8> {
    let trace = recorded_trace(workspace);
    chunk_payloads(&trace)
        .into_iter()
        .find(|(x, z, _)| (*x, *z) == coords)
        .map(|(_, _, bytes)| bytes)
        .unwrap_or_else(|| panic!("chunk ({}, {}) not in the trace", coords.0, coords.1))
}

/// Payload bytes of `FIRST` in the last vanilla run.
pub fn vanilla_payload(workspace: &Path) -> Vec<u8> {
    payload_of(workspace, FIRST)
}

/// Payload bytes of every `level_chunk_with_light` of the last vanilla run,
/// in trace order, with coordinates.
pub fn all_payloads(workspace: &Path) -> Vec<((i32, i32), Vec<u8>)> {
    let trace = recorded_trace(workspace);
    chunk_payloads(&trace)
        .into_iter()
        .map(|(x, z, bytes)| ((x, z), bytes))
        .collect()
}

/// The recorded join trace of the last run.
fn recorded_trace(workspace: &Path) -> Value {
    let text = std::fs::read_to_string(workspace.join("target/parity/join-vanilla.json"))
        .expect("join-vanilla.json (run join_slice first)");
    serde_json::from_str(&text).expect("trace is JSON")
}

/// All recorded chunk payloads with coordinates, skipping malformed ones.
fn chunk_payloads(trace: &Value) -> Vec<(i32, i32, Vec<u8>)> {
    let mut out = Vec::new();
    for packet in trace["packets"].as_array().expect("packets array") {
        if packet["name"] != "minecraft:level_chunk_with_light" {
            continue;
        }
        let bytes = hex_to_bytes(packet["payload"].as_str().expect("payload hex"));
        if bytes.len() < 8 {
            continue;
        }
        let x = i32::from_be_bytes(bytes[0..4].try_into().expect("big endian"));
        let z = i32::from_be_bytes(bytes[4..8].try_into().expect("big endian"));
        out.push((x, z, bytes));
    }
    out
}

/// Stored model of the chunk at `FIRST`, read from `region_dir` (the
/// `dimensions/minecraft/overworld/region` directory of a world copy).
pub fn stored_chunk(region_dir: &Path) -> Chunk {
    stored_chunk_at(region_dir, FIRST.0, FIRST.1).expect("stored chunk of the first batch")
}

/// Stored model of one chunk of a world copy: the region file is picked by
/// the chunk coordinates, the chunk itself by `x`, `z`.
pub fn stored_chunk_at(region_dir: &Path, x: i32, z: i32) -> Option<Chunk> {
    let region = region_dir.join("dimensions/minecraft/overworld/region");
    let file = region.join(format!("r.{}.{}.mca", x >> 5, z >> 5));
    let stored: StoredChunk = world::load_region(&file)
        .ok()?
        .into_iter()
        .find(|chunk| (chunk.x, chunk.z) == (x, z))?;
    Chunk::from_stored(&stored).ok()
}

/// Palette names of one stored section, in palette order. The template
/// stores plain strings and `{ "id": ..., "properties": {...} }` compounds.
pub fn palette_names(section: &world::Section) -> Vec<String> {
    use protocol::NbtTag;
    let Some(NbtTag::Compound(states)) = &section.block_states else {
        return Vec::new();
    };
    let Some(NbtTag::List(palette)) = states.get("palette") else {
        return Vec::new();
    };
    palette
        .iter()
        .filter_map(|entry| match entry {
            NbtTag::String(name) => Some(name.clone()),
            NbtTag::Compound(fields) => {
                ["id", "Name", ""]
                    .iter()
                    .find_map(|key| match fields.get(*key) {
                        Some(NbtTag::String(name)) => Some(name.clone()),
                        _ => None,
                    })
            }
            _ => None,
        })
        .collect()
}

/// Bits per cell of a stored palette with `count` entries: the measured
/// floor of 4 (4 for up to 16 entries, 5 for up to 32, 6 for up to 64).
pub fn bits_for(count: usize) -> usize {
    count.max(1).next_power_of_two().ilog2().max(4) as usize
}

/// The block name in each of the 4096 cells of one stored section, laid
/// out like the wire (`cell = x + z·16 + y·256`); `None` when the palette
/// or the packed data does not resolve.
pub fn stored_cells(section: &world::Section) -> Option<Vec<String>> {
    use protocol::NbtTag;
    let names = palette_names(section);
    if names.is_empty() {
        return None;
    }
    let Some(NbtTag::Compound(states)) = &section.block_states else {
        return None;
    };
    let indices = match states.get("data") {
        None => vec![0_usize; 4096],
        Some(NbtTag::LongArray(words)) => unpack_words(words, bits_for(names.len()))?,
        Some(_) => return None,
    };
    let mut out = Vec::with_capacity(4096);
    for index in indices {
        out.push(names.get(index)?.clone());
    }
    Some(out)
}

/// Unpacks `4096` cell indices out of big-endian words at `bits` per cell
/// (one word holds `64 / bits` cells, cells never span words).
fn unpack_words(words: &[i64], bits: usize) -> Option<Vec<usize>> {
    let per_word = 64 / bits;
    let mask = (u64::MAX >> (64 - bits)) as i64;
    let mut out = Vec::with_capacity(4096);
    for cell in 0..4096_usize {
        let word = *words.get(cell / per_word)?;
        out.push(((word >> ((cell % per_word) * bits)) & mask) as usize);
    }
    Some(out)
}

/// A big-endian `u16` read from `bytes` at `at`.
pub fn be_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

/// A VarInt read from `bytes` at `at`; `None` when it runs past the end.
pub fn read_varint(bytes: &[u8], at: usize) -> Option<(i32, usize)> {
    let mut value = 0_u32;
    let mut taken = 0_usize;
    loop {
        let byte = *bytes.get(at + taken)?;
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
pub fn varint_stream(bytes: &[u8]) -> Vec<i32> {
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
