//! Step 3.2/4 (owner check 3): which stored heightmap each wire record is.
//!
//! `cargo test -p parity --test heightmap_ids -- --ignored --nocapture`
//!
//! The payload opens with a VarInt count of heightmaps and then records
//! `[VarInt id][VarInt 37][37 × i64]`. The owner's instruction was to take
//! the id meaning from `registries.json (heightmap_type)`: this test first
//! proves that the reports carry no registry of that name (checked, not
//! assumed) and then pins every id by byte equality against the
//! `Heightmaps` of the stored chunks: intersecting the keys that byte-match
//! in every chunk leaves one name per id. Records that match no stored
//! map are counted and described, not silently accepted: they are a
//! real wire-vs-template deviation and a parity blocker for
//! encode-equality (step 3.3 f).

#[path = "common/chunk_trace.rs"]
mod chunk_trace;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use chunk_trace::{all_payloads, read_varint, stored_chunk_at};
use protocol::NbtTag;

/// Stored heightmaps of one chunk: key → big-endian bytes of its longs.
fn heightmaps(chunk: &world::Chunk) -> Vec<(String, Vec<u8>)> {
    let NbtTag::Compound(root) = &chunk.root else {
        return Vec::new();
    };
    let Some(NbtTag::Compound(maps)) = root.get("Heightmaps") else {
        return Vec::new();
    };
    maps.iter()
        .filter_map(|(key, value)| match value {
            NbtTag::LongArray(values) => {
                let mut bytes = Vec::with_capacity(values.len() * 8);
                for value in values {
                    bytes.extend_from_slice(&value.to_be_bytes());
                }
                Some((key.clone(), bytes))
            }
            _ => None,
        })
        .collect()
}

/// The recorded heightmap records of all sent chunks: which ids vanilla
/// sends, which stored map every id byte-matches, and which records do not.
#[test]
#[ignore = "requires the owner's files and the recorded trace"]
fn every_heightmap_id_resolves_to_a_stored_map() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let env = parity::load_parity_env().expect("parity environment");

    // The instruction said registries.json: the reports hold no registry
    // of that name, so the ids cannot come from there.
    let registries_path = env.config.registries_report.clone().unwrap_or_default();
    let registries = std::fs::read_to_string(&registries_path).expect("read registries.json");
    assert!(
        !registries.contains("heightmap_type"),
        "no heightmap_type registry exists in the reports (re-run --reports if it appears)"
    );
    eprintln!("reports have no heightmap_type registry: ids resolved by bytes");

    let mut by_id: BTreeMap<i32, BTreeSet<String>> = BTreeMap::new();
    let mut mismatches: Vec<(i32, i32, String, i32)> = Vec::new();
    let mut seen = 0_usize;
    for ((x, z), bytes) in all_payloads(&workspace) {
        let chunk = stored_chunk_at(&env.world_template, x, z).expect("stored chunk");
        let maps = heightmaps(&chunk);
        let mut cursor = 8_usize;
        let (count, len) = read_varint(&bytes, cursor).expect("heightmap count VarInt");
        cursor += len;
        assert!((1..=8).contains(&count), "chunk ({x}, {z}): {count} maps");
        assert_eq!(count, 3, "chunk ({x}, {z}): three heightmap records");
        for _ in 0..count {
            let (id, id_len) = read_varint(&bytes, cursor).expect("map id VarInt");
            cursor += id_len;
            let (longs, len) = read_varint(&bytes, cursor).expect("map length VarInt");
            cursor += len;
            assert_eq!(longs, 37, "chunk ({x}, {z}): {longs} longs per map");
            let end = cursor + longs as usize * 8;
            let data = bytes.get(cursor..end).expect("map bytes");
            cursor = end;
            let hits: Vec<&str> = maps
                .iter()
                .filter(|(_, stored)| stored.as_slice() == data)
                .map(|(key, _)| key.as_str())
                .collect();
            eprintln!(
                "({x}, {z}): id {id} = {}",
                if hits.is_empty() {
                    "NO stored match"
                } else {
                    "byte match"
                }
            );
            if hits.is_empty() {
                // Record the deviation: the closest stored twin and where
                // the bytes start to disagree (9-bit height units, so one
                // differing column shifts one unit inside a long).
                let best = maps
                    .iter()
                    .map(|(key, stored)| {
                        let shared = stored.iter().zip(data).take_while(|(a, b)| a == b).count();
                        (key.clone(), shared)
                    })
                    .max_by_key(|(_, shared)| *shared);
                let detail = best
                    .as_ref()
                    .map(|(key, shared)| format!("{key} shares {shared}/{} bytes", data.len()))
                    .unwrap_or_else(|| "no stored maps".to_owned());
                mismatches.push((x, z, detail, id));
                continue;
            }
            let owned: BTreeSet<String> = hits.iter().map(|hit| hit.to_string()).collect();
            if let Some(keys) = by_id.get_mut(&id) {
                keys.retain(|key| owned.contains(key));
            } else {
                by_id.insert(id, owned);
            }
        }
        assert_eq!(cursor, 903, "chunk ({x}, {z}): maps end before the blob");
        seen += 1;
    }
    assert_eq!(seen, 9, "every sent chunk frames its heightmaps");

    eprintln!("--- id -> stored map the records resolve to (exact matches intersected) ---");
    for (id, keys) in &by_id {
        eprintln!("  id {id}: {keys:?}");
        assert!(!keys.is_empty(), "id {id} matches no stored map");
    }
    assert_eq!(
        by_id.keys().copied().collect::<Vec<_>>(),
        vec![1, 4, 5],
        "the three ids of every sent chunk, in order"
    );
    for id in [1, 4, 5] {
        assert_eq!(
            by_id[&id].len(),
            1,
            "id {id} must resolve to exactly one stored map, got {:?}",
            by_id[&id]
        );
    }
    if !mismatches.is_empty() {
        // Not a parse failure: the wire really differs from the template
        // here. Reported loudly — docs/PROGRESS.md carries this as an open
        // point for step 3.3 (encode-equality would fail on these bytes).
        eprintln!(
            "--- {} wire records match no stored heightmap ---",
            mismatches.len()
        );
        for (x, z, detail, id) in &mismatches {
            eprintln!("  chunk ({x}, {z}) id {id}: {detail}");
        }
    }
}
