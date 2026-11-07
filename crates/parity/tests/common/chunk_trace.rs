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
    let region = region_dir.join("dimensions/minecraft/overworld/region/r.0.-1.mca");
    let stored: StoredChunk = world::load_region(&region)
        .expect("region holding the spawn chunk")
        .into_iter()
        .find(|chunk| (chunk.x, chunk.z) == FIRST)
        .expect("stored chunk in the region");
    Chunk::from_stored(&stored).expect("model")
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
