//! Step 3.2: how every `level_chunk_with_light` of the vanilla join opens.
//!
//! `cargo test -p parity --test chunks_open -- --ignored --nocapture`
//!
//! The payload of a sent chunk starts with `chunkX`, `chunkZ` (big endian)
//! and then a VarInt count of heightmaps: a map is `[VarInt id][VarInt 37]
//! [37 x i64]`. Across all nine sent chunks the framing ends at the same
//! byte, so the bytes that follow are structure shared by every chunk, and
//! the map count is what the raw head bytes `03 04 25` are made of.

#[path = "common/chunk_trace.rs"]
mod chunk_trace;

use std::path::Path;

use chunk_trace::{all_payloads, read_varint};

#[test]
#[ignore = "requires the recorded vanilla join trace"]
fn every_chunk_frames_its_heightmaps_at_the_same_byte() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut chunks = 0_usize;
    for ((x, z), bytes) in all_payloads(&workspace) {
        let mut cursor = 8_usize;
        let Some((count, len)) = read_varint(&bytes, cursor) else {
            panic!("chunk ({x}, {z}): no VarInt of maps after the coordinates");
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
