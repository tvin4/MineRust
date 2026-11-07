//! Step 3.3a: the chunk codecs against the recorded vanilla payloads.
//!
//! `cargo test -p parity --test chunk_codec -- --ignored --nocapture`
//!
//! Criterion (step 3a, `docs/DECISIONS.md`): `encode(decode(bytes))` must
//! be the bytes vanilla sent, for every chunk of the join recording. The
//! light arrays ride along inside those bytes; what the criterion limits
//! is parity against *our* built NBT later — a codec that reproduces the
//! bytes it decoded, light included, is the stronger statement, and the
//! structural light rules (mask vs count, disjoint occupied/empty masks,
//! 2048-byte arrays) are enforced by the decoder itself.
//!
//! WHAT fails here is a codec regression; the known wire-vs-template
//! deviations (seven heightmaps, light content vs stored NBT) are between
//! the trace and the world file, not inside this round trip.

#[path = "common/chunk_trace.rs"]
mod chunk_trace;

use std::path::Path;

use chunk_trace::all_payloads;

#[test]
#[ignore = "requires the owner's files and the recorded trace"]
fn every_recorded_chunk_encodes_back_to_the_recorded_bytes() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut seen = 0_usize;
    for ((x, z), bytes) in all_payloads(&workspace) {
        let mut reader = protocol::PacketReader::new(&bytes);
        let decoded = protocol::LevelChunkWithLight::decode_payload(&mut reader)
            .unwrap_or_else(|err| panic!("chunk ({x}, {z}) decodable: {err}"));
        reader
            .ensure_empty()
            .unwrap_or_else(|err| panic!("chunk ({x}, {z}) fully read: {err}"));

        // Sanity of what the decoder accepted: a chunk of this world holds
        // 24 vertical slots (Y −4..19), three heightmaps and its entities.
        // Every recorded chunk fills all 24 vertical slots (Y −4..19) of this
        // world, as walked by `blob_walk`.
        assert_eq!(decoded.sections.len(), 24, "chunk ({x}, {z}): sections");
        assert_eq!(
            decoded.heightmaps.len(),
            3,
            "chunk ({x}, {z}): three heightmaps"
        );
        // The three ids are stable, their order is not: run 3 of the
        // recording wrote [1, 4, 5] where every other run wrote [4, 5, 1]
        // (vanilla iterates a hash map). The round trip is compared against
        // each recording's own bytes below, never against one fixed order.
        let mut ids: Vec<i32> = decoded.heightmaps.iter().map(|map| map.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 4, 5], "chunk ({x}, {z}): heightmap ids");
        let reencoded = decoded
            .encode_payload()
            .unwrap_or_else(|err| panic!("chunk ({x}, {z}) encodable: {err}"));
        eprintln!(
            "chunk ({x}, {z}): {} bytes, {} sections, {} block entities, \
             sky {} arrays, block {} arrays — roundtrip {}",
            bytes.len(),
            decoded.sections.len(),
            decoded.block_entities.len(),
            decoded.light.sky_arrays.len(),
            decoded.light.block_arrays.len(),
            if reencoded == bytes {
                "byte identical"
            } else {
                "DIFFERS"
            },
        );
        assert_eq!(
            reencoded, bytes,
            "chunk ({x}, {z}): encode(decode(payload)) must be the payload"
        );
        seen += 1;
    }
    assert_eq!(seen, 9, "every chunk of the recording roundtrips");
}

#[test]
#[ignore = "requires the owner's files and the recorded trace"]
fn the_batch_markers_frame_those_chunks() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let trace_path = workspace.join("target/parity/join-vanilla.json");
    let text = std::fs::read_to_string(&trace_path).expect("join-vanilla.json");
    let trace: serde_json::Value = serde_json::from_str(&text).expect("trace is JSON");

    let mut markers = Vec::new();
    for packet in trace["packets"].as_array().expect("packets array") {
        if packet["direction"] != "clientbound" {
            continue;
        }
        let name = packet["name"].as_str().unwrap_or_default();
        if name == "minecraft:chunk_batch_start" || name == "minecraft:chunk_batch_finished" {
            markers.push(packet.clone());
        }
    }
    assert_eq!(markers.len(), 2, "one start and one finished in the run");

    let start = markers.first().expect("batch start first");
    assert_eq!(start["name"], "minecraft:chunk_batch_start", "order");
    assert_eq!(start["payload"], "", "the start carries no fields");

    let finished = markers.last().expect("batch finished last");
    assert_eq!(finished["name"], "minecraft:chunk_batch_finished");
    let recorded = chunk_trace::hex_to_bytes(finished["payload"].as_str().expect("payload hex"));
    let mut reader = protocol::PacketReader::new(&recorded);
    let decoded = protocol::ChunkBatchFinished::decode_payload(&mut reader)
        .expect("batch finished decodable");
    reader.ensure_empty().expect("whole payload read");
    assert_eq!(decoded.chunk_count, 9, "one count per sent chunk");
    assert_eq!(
        decoded.encode_payload(),
        recorded,
        "the batch marker encodes back to the recorded bytes"
    );
}
