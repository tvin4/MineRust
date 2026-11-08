//! Part A2, item 5: why three chunk-diff rows disagree, decided by data.
//!
//! ```text
//! cargo test -p parity --test a2_findings -- --ignored --nocapture
//! ```
//!
//! The three rows of `chunk-diff.md`:
//!
//! * `section non-air/fluid` (5 of 9) — the block cells match 9 of 9, so
//!   only the counting rule can differ. Each candidate rule is computed
//!   from vanilla's **own cells** and checked against vanilla's own
//!   headers, over the join recording and the ack recording (72 chunks).
//! * `heightmap ids (порядок)` (0 of 9) — the id sequence per chunk is
//!   compared between two independent vanilla runs and ours: a stable
//!   vanilla order is our fix, an unstable one goes to DECISIONS.
//! * `block entity records (порядок)` (6 of 9) — both full record lists
//!   per chunk are printed with a verdict on the y-descending key.
//!
//! Output: `target/parity-report/a2-evidence.md` plus the same lines on
//! stdout. This test only produces evidence; it changes nothing.

#[path = "common/a2_data.rs"]
mod a2_data;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use a2_data::{decode, header_evidence, heightmap_order, payloads_of, StateTable};
use protocol::{BlockEntity, LevelChunkWithLight};

/// `(local x, local z, y, type id)` of one record, the shape the diff table
/// compares; NBT is byte-equal everywhere, only the order differs.
type BeRecord = (u8, u8, i16, i32);

/// The absolute workspace root (tests run from the crate directory).
fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Decoded chunks of one recording; a missing file skips with a reason.
fn trace_chunks(file: &str) -> Vec<(i32, i32, LevelChunkWithLight)> {
    let path = workspace().join("target/parity").join(file);
    if !path.is_file() {
        eprintln!("SKIP: recording absent: {}", path.display());
        return Vec::new();
    }
    payloads_of(&path)
        .into_iter()
        .map(|(coords, bytes)| (coords.0, coords.1, decode(coords, &bytes)))
        .collect()
}

/// The reverse block-state table behind `blocks_report` of server.toml.
fn state_table() -> StateTable {
    let env = parity::load_parity_env().expect("parity env (server.toml)");
    let path = env
        .config
        .blocks_report
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(PathBuf::from)
        .expect("blocks_report in server.toml");
    StateTable::load(&path)
}

/// Evidence block 1: vanilla headers vs rules read from vanilla's cells.
fn non_air_fluid_evidence(text: &mut String) -> bool {
    let table = state_table();
    let mut checked = 0_usize;
    let mut non_air_ok = 0_usize;
    let mut name_ok = 0_usize;
    let mut logged_ok = 0_usize;
    let mut full_ok = 0_usize;
    let mut examples = Vec::new();
    let chunks = trace_chunks("join-vanilla.json")
        .into_iter()
        .chain(trace_chunks("chunk-batch-vanilla.json"));
    for (x, z, chunk) in chunks {
        for (index, section) in chunk.sections.iter().enumerate() {
            let got = header_evidence(section, &table);
            checked += 1;
            non_air_ok += usize::from(section.non_air == got.non_air);
            name_ok += usize::from(section.fluid == got.fluid_name);
            logged_ok += usize::from(section.fluid == got.fluid_logged);
            let by_full = section.fluid == got.fluid_full;
            full_ok += usize::from(by_full);
            if !by_full && examples.len() < 12 {
                examples.push(format!(
                    "chunk ({x}, {z}) sec#{index}: vanilla fluid={} vs fluid_full={} \
                     (разница {}): {:?}\n",
                    section.fluid,
                    got.fluid_full,
                    section.fluid as i32 - got.fluid_full as i32,
                    suspicious_names(section, &table)
                ));
            }
        }
    }
    text.push_str("## non-air/fluid: заголовки ванили против правил из её же клеток\n\n");
    text.push_str(&format!(
        "- секций: {checked}; non-air совпал: {non_air_ok}; \
         fluid «water/lava»: {name_ok}; fluid «water/lava + waterlogged=true»: {logged_ok}; \
         fluid «+ aquatic-имена» (fluid_full): {full_ok}\n"
    ));
    for line in &examples {
        text.push_str(&format!("- {line}"));
    }
    if examples.is_empty() {
        text.push_str(
            "- правило «water/lava + waterlogged + seagrass/tall_seagrass/kelp/kelp_plant/\
             bubble_column» совпало с ванилью во всех секциях\n",
        );
    }
    text.push('\n');
    checked == full_ok
}

/// Names of one section with their cell counts — for mismatching sections
/// the histogram shows which block the fluid rule must be counting.
fn suspicious_names(section: &protocol::ChunkSection, table: &StateTable) -> Vec<(String, u32)> {
    let Some(ids) = a2_data::cells(&section.blocks, 4096) else {
        return Vec::new();
    };
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for id in ids {
        if table.is_air(id) {
            continue;
        }
        let name = table
            .get(id)
            .map(|(name, _)| name.to_owned())
            .unwrap_or_else(|| format!("<unknown {id}>"));
        *counts.entry(name).or_default() += 1;
    }
    let mut out: Vec<_> = counts.into_iter().collect();
    out.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    out.truncate(40);
    out
}

/// Evidence block 2: heightmap id order across the vanilla runs.
fn heightmap_evidence(text: &mut String) {
    let files = [
        "join-vanilla.json",
        "join-vanilla.v2runA.json",
        "chunk-batch-vanilla.json",
        "self-session.json",
    ];
    let mut orders: BTreeMap<&str, BTreeMap<(i32, i32), Vec<i32>>> = BTreeMap::new();
    for file in files {
        let path = workspace().join("target/parity").join(file);
        if !path.is_file() {
            eprintln!("SKIP: {file} absent");
            continue;
        }
        let per_coords = payloads_of(&path)
            .into_iter()
            .map(|(coord, bytes)| (coord, heightmap_order(&bytes)))
            .collect();
        orders.insert(file, per_coords);
    }
    text.push_str("## Порядок id карт высот\n\n");
    text.push_str("| файл | последовательности (порядок → кол-во чанков) |\n|---|---|\n");
    for (file, per_coords) in &orders {
        let mut counts: BTreeMap<Vec<i32>, usize> = BTreeMap::new();
        for ids in per_coords.values() {
            *counts.entry(ids.clone()).or_default() += 1;
        }
        let joined = counts
            .iter()
            .map(|(ids, n)| format!("`{ids:?}` ×{n}"))
            .collect::<Vec<_>>()
            .join(", ");
        text.push_str(&format!("| {file} | {joined} |\n"));
    }
    // The two independent join runs on the same template: same chunk,
    // same order — or not, which would make the order unmatchable.
    let (run_a, run_b) = (
        orders.get("join-vanilla.v2runA.json"),
        orders.get("join-vanilla.json"),
    );
    let mut stable = 0_usize;
    let mut conflicts = Vec::new();
    if let (Some(a), Some(b)) = (run_a, run_b) {
        for (coord, ids_a) in a {
            if let Some(ids_b) = b.get(coord) {
                if ids_a == ids_b {
                    stable += 1;
                } else {
                    conflicts.push(format!("{coord:?}: A `{ids_a:?}` vs B `{ids_b:?}`"));
                }
            }
        }
    }
    text.push_str(&format!(
        "\n- одинаковый порядок в двух независимых запусках ванили: {stable} координат, конфликтов: {}\n",
        conflicts.len()
    ));
    for line in conflicts.iter().take(5) {
        text.push_str(&format!("- {line}\n"));
    }
    text.push('\n');
}

/// Evidence block 3: block-entity order, our side from the self session.
fn block_entity_evidence(text: &mut String) {
    let ours: BTreeMap<(i32, i32), LevelChunkWithLight> = trace_chunks("self-session.json")
        .into_iter()
        .map(|(x, z, chunk)| ((x, z), chunk))
        .collect();
    let vanilla = trace_chunks("join-vanilla.json");
    text.push_str("## Порядок записей block entities\n\n");
    // Stability across two independent vanilla runs on the same template.
    let other_run: BTreeMap<(i32, i32), Vec<BeRecord>> = trace_chunks("chunk-batch-vanilla.json")
        .into_iter()
        .map(|(x, z, chunk)| ((x, z), shape(&chunk.block_entities)))
        .collect();
    let (mut same, mut y_desc_ok, mut compared, mut shown) = (0, 0, 0, 0);
    let (mut stable, mut stability_checked) = (0, 0);
    for (x, z, chunk) in &vanilla {
        let Some(ours_chunk) = ours.get(&(*x, *z)) else {
            continue;
        };
        compared += 1;
        let v = shape(&chunk.block_entities);
        let o = shape(&ours_chunk.block_entities);
        same += usize::from(v == o);
        let mut sorted = v.clone();
        sorted.sort_by_key(|(_, _, y, _)| -*y);
        y_desc_ok += usize::from(sorted == v);
        if let Some(other) = other_run.get(&(*x, *z)) {
            if !other.is_empty() {
                stability_checked += 1;
                stable += usize::from(other == &v);
            }
        }
        if v != o && shown < 5 {
            shown += 1;
            text.push_str(&format!("### chunk ({x}, {z})\n\n"));
            text.push_str(&format!("- vanilla: `{v:?}`\n- ours: `{o:?}`\n"));
            text.push_str(&format!(
                "- vanilla отсортирован по y↓: {}\n\n",
                sorted == v
            ));
        }
    }
    text.push_str(&format!(
        "- порядок совпадает с нашим: {same} из {compared}; \
         порядок ванили монотонен по y↓: {y_desc_ok} из {compared}; \
         порядок повторился в двух независимых запусках ванили: {stable} из {stability_checked}\n\n"
    ));
}

/// The record shape the diff table compares.
fn shape(records: &[BlockEntity]) -> Vec<BeRecord> {
    records
        .iter()
        .map(|entity| (entity.local_x, entity.local_z, entity.y, entity.type_id))
        .collect()
}

/// Writes and prints the evidence report.
#[test]
#[ignore = "requires the owner's recordings (join traces, ack trace, self session)"]
fn a2_evidence_report() {
    let mut text = String::from(
        "# Доказательства Части A2, п.5 — по данным трейсов\n\n\
         Генерируется тестом `a2_findings`:\n\
         `cargo test -p parity --test a2_findings -- --ignored --nocapture`.\n\n",
    );
    let header_ok = non_air_fluid_evidence(&mut text);
    heightmap_evidence(&mut text);
    block_entity_evidence(&mut text);
    let report = workspace().join("target/parity-report/a2-evidence.md");
    if let Some(parent) = report.parent() {
        std::fs::create_dir_all(parent).expect("report directory");
    }
    std::fs::write(&report, &text).expect("a2 evidence written");
    println!("{text}");
    eprintln!("a2-evidence written to {}", report.display());
    assert!(
        header_ok,
        "все секции ванили объяснены правилом water/lava + waterlogged + aquatic-имена"
    );
}
