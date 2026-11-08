//! Step 3.3f: the chunk slice — every `level_chunk_with_light` of the join
//! recording against the payload our own world path converts to.
//!
//! ```text
//! cargo test -p parity --test chunk_slice -- --ignored --nocapture
//! ```
//!
//! Vanilla side: the recorded payloads of `target/parity/join-vanilla.json`.
//! Ours: the template world (next to the vanilla install) read through the
//! production path — `server::TemplateWorld` + `server::WireTables`, the
//! structs the loader pool uses, so the table describes what this server
//! would send, not a test double.
//!
//! Output: `target/parity-report/chunk-diff.md` — a per-field table of how
//! many chunks agree plus the first differing example of every field. The
//! test reports, it does not "fix": nothing is equalised before the owner
//! has read the table (rules in `docs/SOURCES.md`, criterion 3a in
//! `docs/DECISIONS.md`).
//!
//! Skipping is graceful: no recording, no template world or no configured
//! report prints `SKIP:` and returns; the test never fabricates a side.

#[path = "common/chunk_compare.rs"]
mod chunk_compare;
#[path = "common/chunk_trace.rs"]
mod chunk_trace;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chunk_compare::compare_pair;
use net::ChunkSource as _;
use parity::{load_parity_env, ParityEnv};
use protocol::LevelChunkWithLight;
use server::{TemplateWorld, WireTables};

/// Field groups of one payload, in report order.
const FIELDS: [&str; 15] = [
    "heightmap ids (порядок)",
    "heightmap words",
    "section count",
    "section non-air/fluid",
    "section block palettes",
    "section biome palettes",
    "block entity count",
    "block entity records (порядок)",
    "block entity NBT bytes",
    "light sky mask",
    "light block mask",
    "light empty-sky mask",
    "light empty-block mask",
    "light sky arrays",
    "light block arrays",
];

/// Longest example text kept in a table cell.
const NOTE_WIDTH: usize = 160;

/// One field group's tally over the paired chunks.
#[derive(Debug, Default, Clone)]
struct Agg {
    /// Chunks where the field was equal.
    equal: usize,
    /// Chunks compared in this group.
    total: usize,
    /// First difference in trace order, kept once.
    diff: Option<String>,
}

type Fields = BTreeMap<&'static str, Agg>;

/// Records one chunk's result for one field group.
fn record(
    fields: &mut Fields,
    field: &'static str,
    coord: (i32, i32),
    equal: bool,
    detail: String,
) {
    let agg = fields.entry(field).or_default();
    agg.total += 1;
    if equal {
        agg.equal += 1;
    } else if agg.diff.is_none() {
        let text = format!("chunk ({}, {}): {detail}", coord.0, coord.1);
        agg.diff = Some(text.chars().take(NOTE_WIDTH).collect());
    }
}

/// Decodes one recorded payload; a recording our codec cannot read fails
/// this slice instead of silently dropping a row.
fn decode_chunk(coords: (i32, i32), bytes: &[u8]) -> LevelChunkWithLight {
    let (x, z) = coords;
    let mut reader = protocol::PacketReader::new(bytes);
    let chunk = LevelChunkWithLight::decode_payload(&mut reader)
        .unwrap_or_else(|err| panic!("chunk ({x}, {z}) decodable: {err}"));
    reader
        .ensure_empty()
        .unwrap_or_else(|err| panic!("chunk ({x}, {z}) fully read: {err}"));
    chunk
}

/// Reads the parity environment or explains why the slice is skipped.
fn parity_env_or_skip() -> Option<ParityEnv> {
    match load_parity_env() {
        Ok(env) => Some(env),
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            None
        }
    }
}

/// A configured report path; `None` (with a printed skip) when the field
/// is absent or empty in `server.toml`.
fn report_path(field: &str, value: Option<&str>) -> Option<PathBuf> {
    match value.map(str::trim).filter(|text| !text.is_empty()) {
        Some(text) => Some(PathBuf::from(text)),
        None => {
            eprintln!("SKIP: `{field}` is not set in server.toml");
            None
        }
    }
}

/// The payloads this server would send for the template world, built by
/// the production conversion path. `None` with a printed reason when a
/// report or the world itself is unavailable on this machine.
fn our_payloads(env: &ParityEnv) -> Option<Vec<LevelChunkWithLight>> {
    let region = env
        .world_template
        .join("dimensions/minecraft/overworld/region");
    if !region.is_dir() {
        eprintln!(
            "SKIP: the template world has no region directory: {}",
            region.display()
        );
        return None;
    }
    let blocks_path = report_path("blocks_report", env.config.blocks_report.as_deref())?;
    let registries_path =
        report_path("registries_report", env.config.registries_report.as_deref())?;
    let blocks = match data::BlockStates::load(&blocks_path) {
        Ok(blocks) => blocks,
        Err(error) => {
            eprintln!("SKIP: cannot read {}: {error}", blocks_path.display());
            return None;
        }
    };
    let registries = match data::load_registries_report(&registries_path) {
        Ok(registries) => registries,
        Err(error) => {
            eprintln!("SKIP: cannot read {}: {error}", registries_path.display());
            return None;
        }
    };
    let game_data =
        match data::load_from_jar(&env.config.client_jar, Some(registries_path.as_path())) {
            Ok(game_data) => game_data,
            Err(error) => {
                eprintln!("SKIP: cannot load game data: {error}");
                return None;
            }
        };
    let level_path = env.world_template.join("level.dat");
    let data_version = match world::read_level_dat(&level_path) {
        Ok(level) => level.data_version,
        Err(error) => {
            eprintln!("SKIP: cannot read {}: {error}", level_path.display());
            return None;
        }
    };
    let world = TemplateWorld::new(
        region,
        data_version,
        Arc::new(WireTables::new(blocks, registries, Arc::new(game_data))),
    );
    match world.load_batch() {
        Ok(chunks) => Some(chunks),
        Err(error) => {
            eprintln!("SKIP: the template world cannot be converted: {error}");
            None
        }
    }
}

/// The finished comparison: everything [`Summary::render`] needs, grouped
/// so rendering never passes more than a few arguments around.
struct Summary {
    /// Recording the vanilla payloads came from.
    trace: PathBuf,
    /// World the own payloads came from.
    template: PathBuf,
    /// Coordinates of the recorded chunks, in trace order.
    vanilla: Vec<(i32, i32)>,
    /// Coordinates of the convertible template chunks.
    ours: Vec<(i32, i32)>,
    /// Chunks found on both sides.
    paired: usize,
    /// Field tallies in [`FIELDS`] order.
    fields: Fields,
}

impl Summary {
    /// The markdown table of the slice.
    fn render(&self) -> String {
        let mut text = String::new();
        text.push_str("# chunk-diff — `level_chunk_with_light`: запись vs наш мир (шаг 3.3f)\n\n");
        text.push_str(
            "Генерируется тестом `chunk_slice`:\n\
             `cargo test -p parity --test chunk_slice -- --ignored --nocapture`.\n\n",
        );
        text.push_str(&format!(
            "- запись: `{}` — {} payload чанков: {:?}\n",
            self.trace.display(),
            self.vanilla.len(),
            self.vanilla
        ));
        text.push_str(&format!(
            "- наша сторона: шаблонный мир `{}` — {} отправляемых чанков\n",
            self.template.display(),
            self.ours.len()
        ));
        text.push_str(&format!(
            "- сопоставление по координатам: {} из {}\n\n",
            self.paired,
            self.vanilla.len()
        ));
        text.push_str("| поле | совпало | расхождений | первое расхождение |\n");
        text.push_str("|---|---|---|---|\n");
        for field in FIELDS {
            let agg = self.fields.get(field).cloned().unwrap_or_default();
            let diff = agg.diff.clone().unwrap_or_else(|| "—".to_owned());
            text.push_str(&format!(
                "| {field} | {} из {} | {} | {} |\n",
                agg.equal,
                agg.total,
                agg.total - agg.equal,
                diff.replace('|', "/")
            ));
        }
        text.push_str(
            "\nНичего не «чинится» по этому отчёту: таблица показана владельцу,\n\
             сама сверка полей решается им (см. `docs/SOURCES.md`, критерий 3а\n\
             в `docs/DECISIONS.md`).\n",
        );
        text
    }
}

/// Step 3.3f: the recorded chunk payloads against the ones this server
/// would send, written out as `target/parity-report/chunk-diff.md`.
#[test]
#[ignore = "requires the owner's parity data (join trace, template world, reports)"]
fn the_chunk_slice_writes_the_diff_table() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let trace_file = workspace.join("target/parity/join-vanilla.json");
    if !trace_file.is_file() {
        eprintln!(
            "SKIP: no join recording at {} (run join_slice first)",
            trace_file.display()
        );
        return;
    }
    let env = match parity_env_or_skip() {
        Some(env) => env,
        None => return,
    };
    let payloads = chunk_trace::all_payloads(&workspace);
    assert!(!payloads.is_empty(), "the recording must hold chunks");
    let vanilla: Vec<(i32, i32)> = payloads.iter().map(|(coords, _)| *coords).collect();
    let chunks: Vec<LevelChunkWithLight> = payloads
        .iter()
        .map(|(coords, bytes)| decode_chunk(*coords, bytes))
        .collect();
    let ours = match our_payloads(&env) {
        Some(ours) => ours,
        None => return,
    };

    let by_coord: BTreeMap<(i32, i32), &LevelChunkWithLight> = ours
        .iter()
        .map(|chunk| ((chunk.chunk_x, chunk.chunk_z), chunk))
        .collect();
    let mut summary = Summary {
        trace: trace_file,
        template: env.world_template.clone(),
        ours: by_coord.keys().copied().collect(),
        vanilla,
        paired: 0,
        fields: FIELDS
            .iter()
            .map(|field| (*field, Agg::default()))
            .collect(),
    };
    let mut unmatched = Vec::new();
    for chunk in &chunks {
        let coord = (chunk.chunk_x, chunk.chunk_z);
        match by_coord.get(&coord) {
            Some(ours_payload) => {
                compare_pair(chunk, ours_payload, &mut summary.fields);
                summary.paired += 1;
            }
            None => unmatched.push(coord),
        }
    }

    let report = workspace.join("target/parity-report/chunk-diff.md");
    if let Some(parent) = report.parent() {
        std::fs::create_dir_all(parent).expect("report directory");
    }
    std::fs::write(&report, summary.render()).expect("chunk-diff table written");
    eprintln!("chunk-diff table written to {}", report.display());
    assert!(
        unmatched.is_empty(),
        "recorded chunks with no convertible own payload: {unmatched:?} (table still written)"
    );
    assert_eq!(
        summary.paired,
        chunks.len(),
        "every recorded chunk compared"
    );
}
