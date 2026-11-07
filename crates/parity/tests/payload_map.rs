//! Step 3.2 groundwork: anchors inside the recorded `level_chunk_with_light`
//! payload.
//!
//! `cargo test -p parity --test payload_map -- --ignored --nocapture`
//!
//! The payload layout of this packet (everything after the two chunk
//! coordinates) is not written down anywhere the agent can reach, so it is
//! derived from two sources that are: the stored chunk NBT (province of the
//! light arrays, the section palettes and the heightmaps) and the raw bytes
//! vanilla sent. Searching the recorded payload for the exact bytes of the
//! stored chunk measures where each field sits on the wire, no guessing
//! involved.

#[path = "common/chunk_trace.rs"]
mod chunk_trace;

use std::path::Path;

use chunk_trace::{all_payloads, stored_chunk, vanilla_payload, FIRST};
use parity::load_parity_env;
use protocol::NbtTag;

/// Prefix lengths probed for an anchor: light is a byte pattern that repeats
/// (`0x00`/`0xff` runs), so short prefixes match everywhere and only a long
/// one can be unique.
const PROBE: [usize; 7] = [32, 64, 128, 256, 512, 1024, 2048];

/// Finds `needle` in `haystack`, returning all byte offsets.
fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() || haystack.len() < needle.len() {
        return out;
    }
    for index in 0..=haystack.len() - needle.len() {
        if haystack[index..index + needle.len()] == *needle {
            out.push(index);
        }
    }
    out
}

/// The longest prefix of `needle` found in `payload`: its length, the first
/// offset carrying it, and how many places carry that prefix.
///
/// The ladder is walked from the longest probe downwards, so a full match
/// wins whenever one exists; a shorter unique prefix is reported only when
/// nothing longer appears.
fn probe(payload: &[u8], needle: &[u8]) -> (usize, Option<usize>, usize) {
    let mut short_unique: Option<(usize, usize)> = None;
    for &length in PROBE.iter().rev() {
        if length > needle.len() {
            continue;
        }
        let hits = find_all(payload, &needle[..length]);
        if hits.is_empty() {
            continue;
        }
        if hits.len() == 1 {
            return (length, Some(hits[0]), 1);
        }
        short_unique.get_or_insert((length, hits[0]));
    }
    short_unique.map_or((0, None, 0), |(length, offset)| {
        let hits = find_all(payload, &needle[..length]).len();
        (length, Some(offset), hits)
    })
}

/// Prints the hit table of every stored light array of `FIRST` and keeps the
/// exact (unique, full-length) matches as layout anchors.
#[test]
#[ignore = "requires the owner's world and the recorded vanilla trace"]
fn locates_the_stored_light_inside_the_recorded_payload() {
    let env = load_parity_env().expect("parity environment");
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let payload = vanilla_payload(&workspace);
    let chunk = stored_chunk(&env.world_template);

    eprintln!(
        "chunk ({}, {}) payload: {} bytes; bytes 8..24: {:02x?}",
        FIRST.0,
        FIRST.1,
        payload.len(),
        &payload[8..24]
    );

    // One row per stored light array: longest prefix found, where, why it
    // stops (first diverging byte), and the exact bytes of both sides at the
    // divergence. This tells a byte-for-byte array from a coordinate shift
    // or a real wire difference.
    eprintln!("--- per-array probe: longest prefix, first divergence ---");
    let mut candidates: Vec<(usize, usize, i8, &'static str, usize)> = Vec::new();
    for (index, section) in chunk.sections.iter().enumerate() {
        for (kind, bytes) in [
            ("BlockLight", section.block_light.as_deref()),
            ("SkyLight", section.sky_light.as_deref()),
        ] {
            let Some(stored) = bytes else { continue };
            let needle: Vec<u8> = stored.iter().copied().map(|byte| byte as u8).collect();
            let (matched, offset, hits) = probe(&payload, &needle);
            eprintln!(
                "sec[{index:>2}] Y={y:>3} {kind:<10} {nbytes} bytes -> prefix {matched} @ {offset:?} hits={hits}",
                y = section.y,
                nbytes = needle.len(),
                offset = offset,
            );
            let Some(offset) = offset else { continue };
            let avail = (payload.len() - offset).min(needle.len());
            let mut diverged = avail;
            for index in 0..avail {
                if payload[offset + index] != needle[index] {
                    diverged = index;
                    break;
                }
            }
            if diverged < avail {
                let lo = offset + diverged;
                eprintln!(
                    "            diverges at relative {diverged}: stored {:02x?} vs payload {:02x?}",
                    &needle[diverged..diverged + 8.min(needle.len() - diverged)],
                    &payload[lo..lo + 8.min(payload.len() - lo)]
                );
            }
            candidates.push((offset, index, section.y, kind, matched));
        }
    }
    candidates.sort();
    eprintln!("--- {} candidates ordered by offset ---", candidates.len());
    for pair in candidates.windows(2) {
        let (prev, ..) = pair[0];
        let (next, ..) = pair[1];
        eprintln!(
            "array @ {prev} -> next @ {next}: stride {} (delta between starts)",
            next as isize - prev as isize
        );
    }

    // The frames around the light region: what stands before the first array
    // (the light masks) and whether the last array ends exactly at the
    // payload end (no trailing fields).
    if let Some(first) = candidates.first().map(|entry| entry.0) {
        let start = first.saturating_sub(40);
        eprintln!(
            "bytes before first array [{start}..{first}]: {:02x?}",
            &payload[start..first]
        );
    }
    if let Some(last) = candidates.last().map(|entry| entry.0) {
        eprintln!(
            "after last array at {last}: {} bytes to payload end",
            payload.len().saturating_sub(last + 2048)
        );
    }

    // Byte frames between anchored arrays: exact separators on the wire.
    eprintln!("--- 8 bytes before / 4 bytes after every full-match anchor ---");
    for &(offset, _, _, _, matched) in &candidates {
        if matched < 2048 {
            continue;
        }
        let start = offset.saturating_sub(8);
        let end = (offset + 2048).min(payload.len());
        let after = (end + 8).min(payload.len());
        eprintln!(
            "anchor {offset}: pre {:02x?} | post {:02x?}",
            &payload[start..offset],
            &payload[end..after]
        );
    }

    // The array of section Y=8: probe small windows all along it to find
    // where (if anywhere) its distinctive middles occur on the wire.
    eprintln!("--- section Y=8 SkyLight: window probes (64 bytes each) ---");
    if let Some(stored) = chunk
        .sections
        .iter()
        .find(|section| section.y == 8)
        .and_then(|section| section.sky_light.as_deref())
    {
        let needle: Vec<u8> = stored.iter().copied().map(|byte| byte as u8).collect();
        for from in [64, 256, 1024, 1536] {
            let hits = find_all(&payload, &needle[from..from + 64]);
            eprintln!(
                "window {from}..{}: {} hits {:?}",
                from + 64,
                hits.len(),
                &hits[..hits.len().min(4)]
            );
        }
    }

    // Every slot of the light region with its content statistics and the
    // stored array it shares the most bytes with: this catches a slot that
    // has no stored twin (light recomputed between the join and the save)
    // and a stored array that was never sent (all zero, omitted on NBT).
    eprintln!("--- full inventory of stored light ---");
    let mut stored_arrays: Vec<(usize, i8, &'static str, Vec<u8>)> = Vec::new();
    for (index, section) in chunk.sections.iter().enumerate() {
        for (kind, bytes) in [
            ("BlockLight", section.block_light.as_deref()),
            ("SkyLight", section.sky_light.as_deref()),
        ] {
            let Some(values) = bytes else { continue };
            stored_arrays.push((
                index,
                section.y,
                kind,
                values.iter().copied().map(|byte| byte as u8).collect(),
            ));
        }
    }
    for (index, y, kind, bytes) in &stored_arrays {
        let non_zero = bytes.iter().filter(|&&byte| byte != 0).count();
        eprintln!(
            "stored sec[{index}] Y={y:>3} {kind} {} bytes, nonzero {non_zero}",
            bytes.len()
        );
    }

    eprintln!("--- wire slots: nonzero bytes and best stored twin ---");
    for slot in [33553, 35603, 37653, 39704, 41754, 43804] {
        if slot + 2048 > payload.len() {
            continue;
        }
        let wire = &payload[slot..slot + 2048];
        let non_zero = wire.iter().filter(|&&byte| byte != 0).count();
        let mut best = (0_usize, "none".to_owned());
        for (index, y, kind, stored) in &stored_arrays {
            let equal = wire.iter().zip(stored).filter(|(a, b)| a == b).count();
            if equal > best.0 {
                best = (equal, format!("sec[{index}] Y={y} {kind}"));
            }
        }
        eprintln!(
            "slot {slot}: nonzero {non_zero}/2048, best twin {} bytes ({})",
            best.0, best.1
        );
    }

    // The masks stand right before the sky group; their bytes plus the group
    // count byte are printed as found, then interpreted by the caller.
    eprintln!(
        "--- region before the sky group: {:02x?}",
        &payload[33502..33556]
    );
    let sky_secs: Vec<i8> = chunk
        .sections
        .iter()
        .filter(|section| section.sky_light.is_some())
        .map(|section| section.y)
        .collect();
    let block_secs: Vec<i8> = chunk
        .sections
        .iter()
        .filter(|section| section.block_light.is_some())
        .map(|section| section.y)
        .collect();
    eprintln!("stored sky sections: {sky_secs:?}; block sections: {block_secs:?}");

    // The structure right after the heightmaps: frames between the long
    // arrays tell how the maps themselves are framed.
    eprintln!("--- heightmap frame bytes ---");
    for (label, range) in [
        ("head 8..11", 8..11),
        ("frame 307..309", 307..309),
        ("frame 605..607", 605..607),
        ("after 903..919", 903..919),
    ] {
        eprintln!("{label}: {:02x?}", &payload[range]);
    }

    // Section block data on the wire: the packed longs of the stored
    // `block_states.data` (BE i64) segment the payload into per-section
    // records and locate the palettes that frame them.
    eprintln!("--- section block data arrays against the payload ---");
    let mut section_hits: Vec<(usize, usize, i8, usize)> = Vec::new();
    for (index, section) in chunk.sections.iter().enumerate() {
        let Some(NbtTag::Compound(states)) = &section.block_states else {
            eprintln!("sec[{index}] Y={:>3}: no block_states compound", section.y);
            continue;
        };
        let palette_len = match states.get("palette") {
            Some(NbtTag::List(items)) => items.len(),
            _ => 0,
        };
        let data: Vec<i64> = match states.get("data") {
            Some(NbtTag::LongArray(values)) => values.clone(),
            _ => Vec::new(),
        };
        if data.is_empty() {
            eprintln!(
                "sec[{index}] Y={:>3}: palette {palette_len} entries, no data (single-valued)",
                section.y
            );
            continue;
        }
        let mut bytes = Vec::with_capacity(data.len() * 8);
        for value in &data {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        let hits = find_all(&payload, &bytes);
        eprintln!(
            "sec[{index}] Y={:>3}: palette {palette_len}, data {} longs -> hits {:?}",
            section.y,
            data.len(),
            hits
        );
        if let Some(&first) = hits.first() {
            section_hits.push((first, index, section.y, palette_len));
        }
    }
    section_hits.sort();
    eprintln!(
        "--- section data order of appearance: {:?}",
        section_hits
            .iter()
            .map(|(offset, index, y, _)| (*offset, *index, *y))
            .collect::<Vec<_>>()
    );

    // Framing of the section records: what stands right before and after a
    // data array (block count, palette headers, data length) and the gap
    // that holds the whole palette of the next section.
    eprintln!("--- section record framing ---");
    if let Some(&(first, index, y, palette_len)) = section_hits.first() {
        eprintln!(
            "sec[{index}] Y={y}: head bytes before data: {:02x?}",
            &payload[first - 16..first]
        );
        let _ = palette_len;
    }
    for pair in section_hits.windows(2) {
        let (end_of_data, ..) = pair[0];
        let (next_data, ..) = pair[1];
        let data_len_hint = 8 * 256; // worst case, recorded to compare with real lengths
        let _ = data_len_hint;
        eprintln!(
            "between data {}..{}: {} bytes {:02x?}",
            end_of_data,
            next_data,
            next_data - end_of_data,
            &payload[end_of_data..next_data.min(end_of_data + 48)]
        );
    }

    // The block entities stored for this chunk are looked for by the ASCII
    // of their ids: if they travel in the packet, their strings are here.
    eprintln!("--- block entity ids as ASCII in the payload ---");
    if let NbtTag::Compound(root) = &chunk.root {
        if let Some(NbtTag::List(items)) = root.get("block_entities") {
            let mut seen: Vec<String> = Vec::new();
            for item in items {
                let NbtTag::Compound(entity) = item else {
                    continue;
                };
                let Some(NbtTag::String(id)) = entity.get("id") else {
                    continue;
                };
                if seen.contains(id) {
                    continue;
                }
                seen.push(id.clone());
                let hits = find_all(&payload, id.as_bytes());
                eprintln!("{id}: {} bytes, ASCII hits at {:?}", id.len(), hits);
            }
            eprintln!("{} block entities stored in NBT", items.len());
        }
    }

    // The 145 bytes between the last packed section and the sky count byte:
    // the single-valued sections, the block entities and the masks must all
    // live here.
    eprintln!("--- tail region before the sky count byte [33405..33556] ---");
    eprintln!("{:02x?}", &payload[33405..33556]);

    // Cross-chunk view: the same prefix after the coordinates in every sent
    // chunk (the owner's 03 04 hypothesis) with its position in each payload.
    eprintln!("--- prefix after x,z in all sent chunks ---");
    for ((x, z), bytes) in all_payloads(&workspace) {
        eprintln!(
            "({x}, {z}) len={} head: {:02x?}",
            bytes.len(),
            &bytes[8..40.min(bytes.len())]
        );
    }

    // The owner's hypothesis: 03 04 25 = «heightmaps: count and type id».
    // The stored chunk carries the heightmaps; if their long arrays sit
    // right after the coordinates, the prefix is decoded by data.
    eprintln!("--- stored heightmaps against the payload head ---");
    if let NbtTag::Compound(root) = &chunk.root {
        if let Some(NbtTag::Compound(maps)) = root.get("Heightmaps") {
            let mut keys: Vec<&String> = maps.keys().collect();
            keys.sort();
            for key in keys {
                let Some(NbtTag::LongArray(values)) = maps.get(key) else {
                    eprintln!("heightmap {key}: not a long array");
                    continue;
                };
                let mut bytes = Vec::with_capacity(values.len() * 8);
                for value in values {
                    bytes.extend_from_slice(&value.to_be_bytes());
                }
                let hits = find_all(&payload, &bytes);
                let in_head: Vec<usize> = find_all(&payload[8..], &bytes)
                    .into_iter()
                    .map(|offset| offset + 8)
                    .collect();
                eprintln!(
                    "heightmap {key}: {} longs ({} bytes), hits at {:?}; starting from byte 8: {:?}",
                    values.len(),
                    bytes.len(),
                    hits,
                    in_head
                );
            }
        }
    }

    // Distinctive middle windows of every stored light array: these avoid
    // the 0x00/0xff runs of the edges and locate an array unambiguously.
    eprintln!("--- middle-window probes (offset 900, 64 bytes) ---");
    for (index, section) in chunk.sections.iter().enumerate() {
        for (kind, bytes) in [
            ("BlockLight", section.block_light.as_deref()),
            ("SkyLight", section.sky_light.as_deref()),
        ] {
            let Some(stored) = bytes else { continue };
            let needle: Vec<u8> = stored.iter().copied().map(|byte| byte as u8).collect();
            let from = 900.min(needle.len() - 64);
            let hits = find_all(&payload, &needle[from..from + 64]);
            eprintln!(
                "sec[{index}] Y={y:>3} {kind:<10} window {from}: hits {hits:?}",
                y = section.y
            );
        }
    }

    // Every `80 10` (VarInt 2048) in the payload: a candidate light array
    // boundary; aligned ones reveal the whole light region.
    eprintln!("--- all VarInt 2048 length prefixes ---");
    let marker = [0x80_u8, 0x10];
    let markers: Vec<usize> = find_all(&payload, &marker)
        .into_iter()
        .filter(|offset| *offset >= 24)
        .collect();
    eprintln!("{markers:?}");
}
