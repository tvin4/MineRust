//! Step 3.2: how every `level_chunk_with_light` of the vanilla join opens.
//!
//! `cargo test -p parity --test chunks_open -- --ignored --nocapture`
//!
//! The payload of a sent chunk starts with `chunkX`, `chunkZ` (big endian)
//! and then a VarInt count of heightmaps: a map is `[VarInt id][VarInt 37]
//! [37 x i64]`. Across all nine sent chunks the framing ends at the same
//! byte, so the bytes that follow are structure shared by every chunk, and
//! the map count is what the raw head bytes `03 04 25` are made of.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Path of the recorded vanilla trace of the last run.
fn trace_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/parity/join-vanilla.json")
}

/// A VarInt read from `bytes` at `at`; `None` when it runs past the end.
fn read_varint(bytes: &[u8], at: usize) -> Option<(i32, usize)> {
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

#[test]
#[ignore = "requires the recorded vanilla join trace"]
fn every_chunk_frames_its_heightmaps_at_the_same_byte() {
    let text = std::fs::read_to_string(trace_path()).expect("join-vanilla.json (run join_slice)");
    let trace: Value = serde_json::from_str(&text).expect("trace is JSON");
    let packets = trace["packets"].as_array().expect("packets array");
    let mut chunks = 0_usize;
    for packet in packets {
        if packet["name"] != "minecraft:level_chunk_with_light" {
            continue;
        }
        let Some(hex) = packet["payload"].as_str() else {
            continue;
        };
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex payload"))
            .collect();
        if bytes.len() < 8 {
            continue;
        }
        let x = i32::from_be_bytes(bytes[0..4].try_into().expect("chunk x"));
        let z = i32::from_be_bytes(bytes[4..8].try_into().expect("chunk z"));
        let mut cursor = 8_usize;
        let Some((count, len)) = read_varint(&bytes, cursor) else {
            continue;
        };
        cursor += len;
        let mut framed = count > 0 && count <= 8;
        for _ in 0..count {
            let Some((_, id_len)) = read_varint(&bytes, cursor) else {
                framed = false;
                break;
            };
            let next = cursor + id_len;
            let Some((longs, long_len)) = read_varint(&bytes, next) else {
                framed = false;
                break;
            };
            if longs != 37 {
                framed = false;
                break;
            }
            cursor = next + long_len + longs as usize * 8;
        }
        assert!(
            framed,
            "chunk ({x}, {z}): heightmaps do not frame at byte {cursor}"
        );
        let rest = bytes.len().saturating_sub(cursor).min(16);
        eprintln!(
            "chunk ({x}, {z}): {count} heightmaps end @ {cursor}; next {rest}: {:02x?}",
            &bytes[cursor..cursor + rest]
        );
        chunks += 1;
    }
    assert!(chunks > 0, "no chunk payloads in the trace");
}
