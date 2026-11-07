//! Step 3.0 of part 3: a read-only inspection of the frozen world template.
//!
//! `cargo test -p parity --test world_inspect -- --ignored --nocapture`
//!
//! Prints, from data only (never from memory):
//!   * the NBT tag trees of `level.dat`, `data/minecraft/world_gen_settings.dat`
//!     and `data/minecraft/game_rules.dat`;
//!   * the inventory of every chunk actually stored in the template's
//!     `dimensions/minecraft/overworld/region/*.mca`: coordinates, sector
//!     compression, `status`, `DataVersion`, sections, block entities and
//!     whether sections carry light arrays;
//!   * the full tag tree of the chunk the vanilla join trace sends first
//!     (5, -2), the observation basis for the "tag -> packet field" mapping.
//!
//! No game logic and no packet encoding happens here. Anything unreadable
//! is reported as an explicit error — the template must never be guessed at.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::Path;

use parity::load_parity_env;
use protocol::NbtTag;

/// Chunk coordinates the vanilla join trace sends as its first `level_chunk_with_light`.
const FIRST_SENT_CHUNK: (i32, i32) = (5, -2);

/// One stored chunk: coordinates and the decompressed disk NBT.
struct StoredChunk {
    /// Chunk x in world coordinates.
    x: i32,
    /// Chunk z in world coordinates.
    z: i32,
    /// Sector compression byte of the region file (1 = gzip, 2 = zlib, 3 = raw).
    compression: u8,
    /// Decompressed root NBT compound.
    nbt: NbtTag,
}

/// Decodes the disk NBT form `type + root name + payload`.
///
/// `protocol::NbtTag` decodes the network form (no root name), so the name
/// is skipped and the type byte re-prefixed for the network decoder.
fn disk_nbt(bytes: &[u8]) -> Result<NbtTag, String> {
    let type_id = *bytes.first().ok_or("empty nbt input")?;
    if type_id != 10 {
        return Err(format!("root tag type {type_id} is not a compound"));
    }
    let name_len = bytes
        .get(1..3)
        .map(|len| u16::from_be_bytes([len[0], len[1]]) as usize)
        .ok_or("truncated root name length")?;
    let payload = bytes.get(3 + name_len..).ok_or("truncated root name")?;
    let mut network = Vec::with_capacity(payload.len() + 1);
    network.push(type_id);
    network.extend_from_slice(payload);
    let (tag, _) = NbtTag::decode_network(&network).map_err(|error| error.to_string())?;
    Ok(tag)
}

/// Decompresses one region sector by its compression byte.
fn decompress(kind: u8, data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let result = match kind {
        1 => flate2::read::GzDecoder::new(data).read_to_end(&mut out),
        2 => flate2::read::ZlibDecoder::new(data).read_to_end(&mut out),
        3 => {
            out.extend_from_slice(data);
            Ok(0)
        }
        other => return Err(format!("unknown region compression type {other}")),
    };
    result.map_err(|error| format!("decompression failed: {error}"))?;
    Ok(out)
}

/// Region coordinates from a filename like `r.-1.0.mca`.
fn region_coords(path: &Path) -> Result<(i32, i32), String> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or("bad name")?;
    let parts: Vec<&str> = stem
        .strip_prefix("r.")
        .ok_or("bad region name")?
        .split('.')
        .collect();
    let x = parts
        .first()
        .and_then(|v| v.parse().ok())
        .ok_or("bad region x")?;
    let z = parts
        .get(1)
        .and_then(|v| v.parse().ok())
        .ok_or("bad region z")?;
    Ok((x, z))
}

/// Reads every present chunk of one `.mca` file through the 4 KiB header.
fn read_region(path: &Path) -> Result<Vec<StoredChunk>, String> {
    let file = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    if file.len() < 8192 {
        return Err(format!("{}: shorter than a header", path.display()));
    }
    let (region_x, region_z) = region_coords(path)?;
    let mut chunks = Vec::new();
    for index in 0..1024usize {
        let entry = &file[index * 4..index * 4 + 4];
        if entry[3] == 0 {
            continue; // absent chunk slot
        }
        let header =
            (usize::from(entry[0]) << 16) | (usize::from(entry[1]) << 8) | usize::from(entry[2]);
        let offset = header * 4096;
        if offset + 5 > file.len() {
            return Err(format!(
                "{} slot {index}: offset beyond file",
                path.display()
            ));
        }
        let length = u32::from_be_bytes([
            file[offset],
            file[offset + 1],
            file[offset + 2],
            file[offset + 3],
        ]) as usize;
        let body = file
            .get(offset + 5..offset + 4 + length)
            .ok_or("sector body beyond file")?;
        let raw = decompress(file[offset + 4], body)?;
        chunks.push(StoredChunk {
            x: region_x * 32 + (index % 32) as i32,
            z: region_z * 32 + (index / 32) as i32,
            compression: file[offset + 4],
            nbt: disk_nbt(&raw)?,
        });
    }
    Ok(chunks)
}

/// A compound child's string value, if present.
fn str_of<'a>(tag: &'a NbtTag, key: &str) -> Option<&'a str> {
    match tag {
        NbtTag::Compound(map) => match map.get(key) {
            Some(NbtTag::String(value)) => Some(value),
            _ => None,
        },
        _ => None,
    }
}

/// A compound child's integer value, if present.
fn int_of(tag: &NbtTag, key: &str) -> Option<i64> {
    match tag {
        NbtTag::Compound(map) => match map.get(key) {
            Some(NbtTag::Int(value)) => Some(i64::from(*value)),
            Some(NbtTag::Long(value)) => Some(*value),
            _ => None,
        },
        _ => None,
    }
}

/// Prints `path/to/file.dat (N bytes) -> tag tree` with arrays truncated.
fn print_dat(root: &Path, relative: &str) {
    let path = root.join(relative);
    let raw = match fs::read(&path) {
        Ok(raw) => raw,
        Err(error) => {
            eprintln!("  {relative}: cannot read ({error})");
            return;
        }
    };
    let nbt = match run_gunzip(&raw) {
        Ok(tag) => tag,
        Err(error) => {
            eprintln!("  {relative}: DECOMPRESS ERROR: {error}");
            return;
        }
    };
    eprintln!("  {relative} ({} B):", raw.len());
    print_tree("    ", &nbt, 0);
}

/// Gunzips and decodes one `.dat` file's bytes.
fn run_gunzip(raw: &[u8]) -> Result<NbtTag, String> {
    let mut decoded = Vec::new();
    flate2::read::GzDecoder::new(raw)
        .read_to_end(&mut decoded)
        .map_err(|error| format!("gzip: {error}"))?;
    disk_nbt(&decoded)
}

/// Depth-limited pretty printer; arrays show their length and first values.
fn print_tree(indent: &str, tag: &NbtTag, depth: usize) {
    if depth > 6 {
        eprintln!("{indent}...");
        return;
    }
    match tag {
        NbtTag::Compound(map) => {
            for (name, value) in map {
                match value {
                    NbtTag::Compound(_) | NbtTag::List(_) => {
                        eprintln!("{indent}{name}:");
                        print_tree(&format!("{indent}  "), value, depth + 1);
                    }
                    other => eprintln!("{indent}{name} = {}", scalar(other)),
                }
            }
        }
        NbtTag::List(items) => {
            eprintln!("{indent}[{} items]", items.len());
            if let Some(first) = items.first() {
                print_tree(&format!("{indent}  "), first, depth + 1);
            }
        }
        other => eprintln!("{indent}{}", scalar(other)),
    }
}

/// One-line rendering of a scalar NBT value.
fn scalar(tag: &NbtTag) -> String {
    match tag {
        NbtTag::Byte(v) => format!("{v} (byte)"),
        NbtTag::Short(v) => format!("{v} (short)"),
        NbtTag::Int(v) => format!("{v} (int)"),
        NbtTag::Long(v) => format!("{v} (long)"),
        NbtTag::Float(v) => format!("{v} (float)"),
        NbtTag::Double(v) => format!("{v} (double)"),
        NbtTag::String(v) => format!("\"{v}\""),
        NbtTag::ByteArray(v) => format!("[{} bytes] {:?}", v.len(), &v[..v.len().min(8)]),
        NbtTag::IntArray(v) => format!("[{} ints] {v:?}", v.len().min(16)),
        NbtTag::LongArray(v) => format!("[{} longs] first: {v:?}", v.len().min(8)),
        NbtTag::List(items) => format!("[{} items]", items.len()),
        NbtTag::Compound(_) => String::from("{...}"),
    }
}

/// The inspection itself: prints trees, inventories and a summary.
#[test]
#[ignore = "step 3.0: reads C:/MineRust-data, prints the world template"]
fn inspect_world_template() {
    let env = match load_parity_env() {
        Ok(env) => env,
        Err(error) => panic!("parity env: {error}"),
    };
    let root = &env.world_template;
    eprintln!("=== world template: {} ===", root.display());

    // --- small data files: full tag trees ---
    for relative in [
        "level.dat",
        "data/minecraft/world_gen_settings.dat",
        "data/minecraft/game_rules.dat",
    ] {
        print_dat(root, relative);
    }

    // --- region inventory: every chunk actually stored ---
    let region_dir = root.join("dimensions/minecraft/overworld/region");
    let mut entries: Vec<_> = fs::read_dir(&region_dir)
        .unwrap_or_else(|error| panic!("{}: {error}", region_dir.display()))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "mca"))
        .collect();
    entries.sort();

    let mut all: BTreeMap<(i32, i32), (u8, NbtTag)> = BTreeMap::new();
    for path in &entries {
        let chunks = read_region(path).unwrap_or_else(|error| panic!("{error}"));
        eprintln!(
            "\n--- {} ({} chunks) ---",
            path.file_name().expect("name").to_string_lossy(),
            chunks.len()
        );
        for chunk in &chunks {
            // The stored key is capitalized: `Status`, not `status`.
            let status = str_of(&chunk.nbt, "Status").unwrap_or("<no Status>");
            let version = int_of(&chunk.nbt, "DataVersion").unwrap_or(-1);
            let sections = match &chunk.nbt {
                NbtTag::Compound(map) => match map.get("sections") {
                    Some(NbtTag::List(items)) => items.len(),
                    _ => 0,
                },
                _ => 0,
            };
            let block_entities = match &chunk.nbt {
                NbtTag::Compound(map) => match map.get("block_entities") {
                    Some(NbtTag::List(items)) => items.len(),
                    _ => 0,
                },
                _ => 0,
            };
            eprintln!(
                "  ({:>3},{:>3}) compression={} status=\"{status}\" DataVersion={version} sections={sections} block_entities={block_entities}",
                chunk.x, chunk.z, chunk.compression
            );
            all.insert((chunk.x, chunk.z), (chunk.compression, chunk.nbt.clone()));
        }
    }

    // --- summary ---
    let xs: BTreeSet<i32> = all.keys().map(|key| key.0).collect();
    let zs: BTreeSet<i32> = all.keys().map(|key| key.1).collect();
    eprintln!(
        "\n=== summary: {} chunks, x in [{}, {}], z in [{}, {}] ===",
        all.len(),
        xs.first().expect("x"),
        xs.last().expect("x"),
        zs.first().expect("z"),
        zs.last().expect("z")
    );

    // The chunk the vanilla join trace sends first: full tree.
    let (fx, fz) = FIRST_SENT_CHUNK;
    match all.get(&(fx, fz)) {
        Some((compression, nbt)) => {
            eprintln!("\n--- first sent chunk ({fx}, {fz}) compression={compression}: ---");
            print_tree("  ", nbt, 0);
        }
        None => eprintln!("\nfirst sent chunk ({fx}, {fz}): NOT STORED in the template"),
    }

    // Light arrays and `Status`: which chunks store them at all?
    let mut lit_chunks: Vec<(i32, i32)> = Vec::new();
    let mut statuses: BTreeMap<String, usize> = BTreeMap::new();
    for ((x, z), (_, nbt)) in &all {
        let status = str_of(nbt, "Status").unwrap_or("<no Status>").to_owned();
        *statuses.entry(status).or_default() += 1;
        if let NbtTag::Compound(map) = nbt {
            if let Some(NbtTag::List(sections)) = map.get("sections") {
                let lit = sections.iter().any(|section| {
                    matches!(section, NbtTag::Compound(inner)
                        if inner.contains_key("BlockLight") || inner.contains_key("SkyLight"))
                });
                if lit {
                    lit_chunks.push((*x, *z));
                }
            }
        }
    }
    eprintln!(
        "\nchunks whose sections carry BlockLight/SkyLight: {}/{}",
        lit_chunks.len(),
        all.len()
    );
    eprintln!("  {lit_chunks:?}");
    eprintln!("Status counts: {statuses:?}");

    // Reports needed by later steps (block state ids, registries), located
    // next to the configured packets report.
    if let Some(packets) = env.config.packets_report.as_deref() {
        let dir = Path::new(packets).parent().unwrap_or(Path::new("."));
        eprintln!("\nreports dir (from packets_report): {}", dir.display());
        for report in ["blocks.json", "registries.json", "packets.json"] {
            match fs::metadata(dir.join(report)) {
                Ok(meta) => eprintln!("  {report}: {} B", meta.len()),
                Err(error) => eprintln!("  {report}: MISSING ({error})"),
            }
        }
    }

    assert!(!all.is_empty(), "template stored no chunks");
    assert!(all.contains_key(&(fx, fz)), "first sent chunk missing");
}
