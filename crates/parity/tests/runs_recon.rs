//! Diagnosis of owner step 3: the four recorded joins side by side.
//!
//! The recorded world of `join-vanilla.json` differs from the served
//! template in eight of nine chunks (block data), and (7, −2) carries two
//! block entity records the stored `block_entities` list does not hold —
//! before touching the recording (it is evidence) every archived run is
//! read to learn whether the deviation is stable across runs.
//!
//! ```text
//! cargo test -p parity --test runs_recon -- --ignored --nocapture
//! ```

#[path = "common/chunk_trace.rs"]
mod chunk_trace;

use std::collections::BTreeMap;
use std::path::Path;

/// One section seen from three sides: section index, the cells of the wire
/// recording, of the served template and of the world vanilla saved.
type SectionCells = (usize, Vec<String>, Vec<String>, Vec<String>);

/// One archived recording of a vanilla join.
const RUNS: [&str; 4] = [
    "join-vanilla.json",
    "join-vanilla.run1.json",
    "join-vanilla.run3.json",
    "join-vanilla.run4.json",
];

/// The nine ready chunks of the parity world around the spawn.
const NINE: [(i32, i32); 9] = [
    (6, -2),
    (6, -1),
    (7, -2),
    (5, -2),
    (6, -3),
    (7, -1),
    (5, -1),
    (5, -3),
    (7, -3),
];

#[test]
#[ignore = "requires the archived join recordings in target/parity"]
fn the_phantom_records_stay_the_same_across_the_archived_runs() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut fingerprints: Vec<(&str, Vec<u8>)> = Vec::new();
    for file in RUNS {
        let text = match std::fs::read_to_string(workspace.join("target/parity").join(file)) {
            Ok(text) => text,
            Err(_) => {
                eprintln!("{file}: not readable, skipped");
                continue;
            }
        };
        let trace: serde_json::Value = serde_json::from_str(&text).expect("trace is JSON");
        let mut chunks = 0_usize;
        let mut phantom = None;
        let mut first = None;
        let mut coords: Vec<(i32, i32)> = Vec::new();
        for packet in trace["packets"].as_array().expect("packets array") {
            if packet["name"] != "minecraft:level_chunk_with_light" {
                continue;
            }
            chunks += 1;
            let bytes = chunk_trace::hex_to_bytes(packet["payload"].as_str().expect("hex"));
            if bytes.len() < 8 {
                continue;
            }
            let coord = (
                i32::from_be_bytes(bytes[0..4].try_into().expect("be")),
                i32::from_be_bytes(bytes[4..8].try_into().expect("be")),
            );
            coords.push(coord);
            if first.is_none() {
                first = Some((coord, fingerprint(&bytes)));
            }
            if coord == (7, -2) {
                phantom = Some(decode_count(&bytes));
            }
        }
        eprintln!(
            "{file}: {chunks} chunks, first {first:?}, (7, -2) block entities: {phantom:?}; \
             coordinates {coords:?}"
        );
        if let Some(bytes) = take_payload(&trace, (6, -2)) {
            fingerprints.push((file, bytes));
        }
    }
    for index in 1..fingerprints.len().min(4) {
        let (file_one, one) = &fingerprints[0];
        let (file_two, two) = &fingerprints[index];
        eprintln!("comparing (6, -2) of {file_one} against {file_two}");
        compare_payloads(one, two);
    }
}

/// The world vanilla saved right after the recorded join: every one of the
/// nine chunks against the template it was copied from and against the wire
/// recording. `block_entities` of (7, −2) decides whether vanilla *loaded*
/// two records (the template really held them then) or invented them in
/// memory — and the section counts say how far the served world ever sat
/// from the frozen template.
#[test]
#[ignore = "requires the owner's vanilla installation and the recording"]
fn the_persisted_world_is_what_the_recording_was_made_from() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let env = parity::load_parity_env().expect("parity environment");
    let vanilla_world = env.vanilla_dir.join("world");
    for coord in NINE {
        let wire = chunk_trace::payload_of(&workspace, coord);
        let mut reader = protocol::PacketReader::new(&wire);
        let Some(recorded) = protocol::LevelChunkWithLight::decode_payload(&mut reader).ok() else {
            eprintln!("chunk {coord:?}: recording does not decode");
            continue;
        };
        let saved = chunk_trace::stored_chunk_at(&vanilla_world, coord.0, coord.1);
        let templ = chunk_trace::stored_chunk_at(&env.world_template, coord.0, coord.1);
        let (saved, templ) = match (saved, templ) {
            (Some(saved), Some(templ)) => (saved, templ),
            pair => {
                eprintln!(
                    "chunk {coord:?}: saved/template missing: {}",
                    pair.0.is_some()
                );
                continue;
            }
        };
        let vs_template = differing_sections(&saved, &templ);
        let vs_wire = wire_differing(&recorded, &saved);
        eprintln!(
            "chunk {coord:?}: BE saved {} / template {} / wire {}; \
             sections saved-vs-template {} of {}, saved-vs-wire {} of {}",
            be_count(&saved.root),
            be_count(&templ.root),
            recorded.block_entities.len(),
            vs_template,
            saved.sections.len(),
            vs_wire,
            saved.sections.len()
        );
        if coord == (7, -2) {
            for entry in saved_entities(&saved.root) {
                eprintln!(
                    "  saved (7,-2) entry: x {} y {} z {} id {}",
                    debug_field(&entry, "x"),
                    debug_field(&entry, "y"),
                    debug_field(&entry, "z"),
                    debug_field(&entry, "id"),
                );
            }
        }
    }
}

/// Every cell of the nine chunks resolved to a block name on three sides:
/// the wire recording (state ids → `blocks.json`), the served template and
/// the world vanilla saved after the run. Raw data words cannot be
/// compared directly (the two palettes order their entries independently),
/// so only the resolved names say whether the blocks actually differ —
/// the cells the two phantom records of (7, −2) sit on included.
#[test]
#[ignore = "requires the owner's files, reports and the recording"]
fn cells_of_the_recording_the_template_and_the_saved_world() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let env = parity::load_parity_env().expect("parity environment");
    let blocks_path = env
        .config
        .blocks_report
        .as_deref()
        .expect("blocks_report path");
    let blocks = data::BlockStates::load(blocks_path).expect("blocks report reads");
    let vanilla_world = env.vanilla_dir.join("world");

    for coord in NINE {
        let (Some(wire), Some(template), Some(saved)) = (
            decode_chunk(&workspace, coord),
            chunk_trace::stored_chunk_at(&env.world_template, coord.0, coord.1),
            chunk_trace::stored_chunk_at(&vanilla_world, coord.0, coord.1),
        ) else {
            eprintln!("chunk {coord:?}: a side is missing");
            continue;
        };
        let mut wire_vs_template = 0_usize;
        let mut wire_vs_saved = 0_usize;
        let mut resolved: Vec<SectionCells> = Vec::new();
        for (index, recorded) in wire.sections.iter().enumerate() {
            let (Some(recorded_cells), Some(template_cells), Some(saved_cells)) = (
                wire_cells(&recorded.blocks, &blocks),
                chunk_trace::stored_cells(&template.sections[index]),
                chunk_trace::stored_cells(&saved.sections[index]),
            ) else {
                eprintln!("  section #{index}: a side does not resolve");
                continue;
            };
            wire_vs_template += differing(&recorded_cells, &template_cells);
            wire_vs_saved += differing(&recorded_cells, &saved_cells);
            resolved.push((index, recorded_cells, template_cells, saved_cells));
        }
        eprintln!(
            "chunk {coord:?}: {} sections, cells differ wire-vs-template {wire_vs_template}, \
             wire-vs-saved {wire_vs_saved}",
            resolved.len()
        );
        if coord == (7, -2) {
            report_phantom_details(&template, &resolved);
        }
    }
}

/// The (7, −2) close-up: which sections carry the wire-vs-template cell
/// differences, which block replacements are most common, and what the two
/// phantom record coordinates hold on each of the three sides.
fn report_phantom_details(template: &world::Chunk, resolved: &[SectionCells]) {
    let mut steps: BTreeMap<(String, String), usize> = BTreeMap::new();
    for (index, wire, templ, _saved) in resolved {
        let moves = differing(wire, templ);
        if moves > 0 {
            eprintln!(
                "  section #{index} (Y={}): {moves} cells differ",
                template_y(template, *index)
            );
        }
        for (wire_cell, template_cell) in wire.iter().zip(templ) {
            if wire_cell != template_cell {
                *steps
                    .entry((template_cell.clone(), wire_cell.clone()))
                    .or_default() += 1;
            }
        }
    }
    eprintln!("  template -> wire steps by frequency:");
    let mut ordered: Vec<_> = steps.into_iter().collect();
    ordered.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    for ((from, to), count) in ordered.into_iter().take(12) {
        eprintln!("    {from} -> {to}: {count}");
    }
    for (x, y, z) in [(121, -40, -23), (124, -40, -20)] {
        report_cell_sides(template, resolved, x, y, z);
    }
}

/// One coordinate of the two phantom records: the block each side holds.
fn report_cell_sides(template: &world::Chunk, resolved: &[SectionCells], x: i32, y: i32, z: i32) {
    let Some((index, cell)) = cell_of(template, x, y, z) else {
        eprintln!("  ({x}, {y}, {z}): section not found");
        return;
    };
    let Some((_, wire, templ, saved)) = resolved.iter().find(|(i, ..)| *i == index) else {
        eprintln!("  ({x}, {y}, {z}): section #{index} unresolved");
        return;
    };
    eprintln!(
        "  ({x}, {y}, {z}): wire {:?}, template {:?}, saved {:?}",
        wire.get(cell),
        templ.get(cell),
        saved.get(cell)
    );
}

/// Section index and cell index of one world coordinate of a chunk.
fn cell_of(chunk: &world::Chunk, x: i32, y: i32, z: i32) -> Option<(usize, usize)> {
    let index = chunk
        .sections
        .iter()
        .position(|section| i32::from(section.y) == y.div_euclid(16))?;
    let local = |axis: i32| axis.rem_euclid(16);
    let cell = local(x) + local(z) * 16 + local(y) * 256;
    Some((index, usize::try_from(cell).ok()?))
}

/// The world y of one section index (for readable labels).
fn template_y(chunk: &world::Chunk, index: usize) -> i32 {
    chunk
        .sections
        .get(index)
        .map(|section| i32::from(section.y))
        .unwrap_or_default()
}

/// The stored model behind one recorded payload, `None` when the payload is
/// not in the archive.
fn decode_chunk(workspace: &Path, coord: (i32, i32)) -> Option<protocol::LevelChunkWithLight> {
    let bytes = chunk_trace::payload_of(workspace, coord);
    let mut reader = protocol::PacketReader::new(&bytes);
    protocol::LevelChunkWithLight::decode_payload(&mut reader).ok()
}

/// Names in every cell of one wire section, resolved through `blocks.json`.
fn wire_cells(
    blocks: &protocol::PaletteContainer,
    report: &data::BlockStates,
) -> Option<Vec<String>> {
    let ids = match blocks {
        protocol::PaletteContainer::Single(id) => vec![*id; 4096],
        protocol::PaletteContainer::Packed {
            bits,
            palette,
            data,
        } => {
            let bits = *bits as usize;
            let per_word = 64 / bits;
            let mask = u64::MAX >> (64 - bits);
            let mut out = Vec::with_capacity(4096);
            for cell in 0..4096_usize {
                let start = cell / per_word * 8;
                let word = u64::from_be_bytes(data.get(start..start + 8)?.try_into().ok()?);
                let index = ((word >> ((cell % per_word) * bits)) & mask) as usize;
                out.push(*palette.get(index)?);
            }
            out
        }
    };
    ids.iter()
        .map(|id| report.block_name(*id).map(str::to_owned))
        .collect()
}

/// Cells holding a different name in two equal-length cell lists.
fn differing(one: &[String], other: &[String]) -> usize {
    one.iter().zip(other).filter(|(a, b)| a != b).count()
}

/// Sections whose packed block data differs, counted against the other chunk.
fn differing_sections(one: &world::Chunk, other: &world::Chunk) -> usize {
    one.sections
        .iter()
        .zip(&other.sections)
        .filter(|(a, b)| stored_data_bytes(a) != stored_data_bytes(b))
        .count()
}

/// Sections whose packed block data differs between the recording and the
/// stored chunk (a single-valued wire palette has no data words at all).
fn wire_differing(recorded: &protocol::LevelChunkWithLight, stored: &world::Chunk) -> usize {
    recorded
        .sections
        .iter()
        .zip(&stored.sections)
        .filter(|(wire, mine)| wire_data(&wire.blocks) != stored_data_bytes(mine))
        .count()
}

/// Packed data words of a wire palette container, empty when single-valued.
fn wire_data(blocks: &protocol::PaletteContainer) -> Vec<u8> {
    match blocks {
        protocol::PaletteContainer::Single(_) => Vec::new(),
        protocol::PaletteContainer::Packed { data, .. } => data.clone(),
    }
}

/// The packed block data of a stored section: NBT longs big-endian.
fn stored_data_bytes(section: &world::Section) -> Vec<u8> {
    let Some(protocol::NbtTag::Compound(states)) = &section.block_states else {
        return Vec::new();
    };
    match states.get("data") {
        Some(protocol::NbtTag::LongArray(words)) => {
            words.iter().flat_map(|word| word.to_be_bytes()).collect()
        }
        _ => Vec::new(),
    }
}

/// Number of compounds in the root's `block_entities` list.
fn be_count(root: &protocol::NbtTag) -> usize {
    saved_entities(root).len()
}

/// The `block_entities` compounds of a chunk root.
fn saved_entities(
    root: &protocol::NbtTag,
) -> Vec<std::collections::BTreeMap<String, protocol::NbtTag>> {
    let protocol::NbtTag::Compound(map) = root else {
        return Vec::new();
    };
    match map.get("block_entities") {
        Some(protocol::NbtTag::List(items)) => items
            .iter()
            .filter_map(|item| match item {
                protocol::NbtTag::Compound(entry) => Some(entry.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Debug string of an optional NBT field.
fn debug_field(entry: &std::collections::BTreeMap<String, protocol::NbtTag>, key: &str) -> String {
    entry
        .get(key)
        .map(|tag| format!("{tag:?}"))
        .unwrap_or_else(|| "<absent>".to_owned())
}

/// Payload of the chunk at `coord`, `None` when the run has none.
fn take_payload(trace: &serde_json::Value, coord: (i32, i32)) -> Option<Vec<u8>> {
    trace["packets"].as_array()?.iter().find_map(|packet| {
        if packet["name"] != "minecraft:level_chunk_with_light" {
            return None;
        }
        let bytes = chunk_trace::hex_to_bytes(packet["payload"].as_str()?);
        let head = (
            i32::from_be_bytes(bytes.get(0..4)?.try_into().ok()?),
            i32::from_be_bytes(bytes.get(4..8)?.try_into().ok()?),
        );
        (head == coord).then_some(bytes)
    })
}

/// Field-by-field: which group of the payload differs between two runs.
fn compare_payloads(one: &[u8], two: &[u8]) {
    if one == two {
        eprintln!("(6, -2): payloads byte-identical");
        return;
    }
    let decode = |bytes: &[u8]| {
        let mut reader = protocol::PacketReader::new(bytes);
        protocol::LevelChunkWithLight::decode_payload(&mut reader).expect("decodable")
    };
    let (a, b) = (decode(one), decode(two));
    eprintln!(
        "(6, -2): payloads differ; heightmaps equal {}, sections equal {}, \
         block entities equal {}, light equal {}, whole {} vs {} bytes",
        a.heightmaps == b.heightmaps,
        a.sections == b.sections,
        a.block_entities == b.block_entities,
        a.light == b.light,
        one.len(),
        two.len()
    );
    let first = one
        .iter()
        .zip(two)
        .position(|(x, y)| x != y)
        .unwrap_or_default();
    eprintln!("  first differing byte at {first}");
}

/// Count of block entity records of a decoded payload, `None` when it does
/// not decode (the finding itself, never a panic here).
fn decode_count(bytes: &[u8]) -> Option<usize> {
    let mut reader = protocol::PacketReader::new(bytes);
    let decoded = protocol::LevelChunkWithLight::decode_payload(&mut reader).ok()?;
    Some(decoded.block_entities.len())
}

/// A short hash of the payload as a whole: identical prints equal worlds.
fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
