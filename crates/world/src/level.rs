//! Reading the world documents: `level.dat` and `world_gen_settings.dat`.
//!
//! SOURCE: the owner's template — both files were walked byte by byte: a
//! gzip container around the disk NBT form of [`crate::nbt::decode_disk`].
//! `level.dat`'s root compound holds `Data` with the `DataVersion` integer
//! the send rule compares every chunk against (step 3.3b) and the `spawn`
//! compound (`pos` block coordinates); `data/minecraft/world_gen_settings.dat`
//! holds the `seed` the client is introduced to through `hashed_seed`
//! (step 3.3e).
//!
//! The data-pack documents the game saves (`world_gen_settings.dat`,
//! `game_rules.dat`, ...) are wrapped in a `data` compound with the
//! `DataVersion` int beside it at the root — walked byte by byte on the
//! owner's template. The seed reader therefore looks under `data` first and
//! falls back to a bare root `seed` (the shape older writers produce);
//! both forms are covered by tests, the real file by an ignored one
//! (`tests/template_docs.rs`).

use std::fs;
use std::io::Read;
use std::path::Path;

use flate2::read::GzDecoder;
use protocol::{NbtTag, Position};

use crate::error::WorldError;
use crate::nbt::decode_disk;

/// The fields of `level.dat` the server has a use for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelDat {
    /// `Data.DataVersion`: the schema every sent chunk of the save must
    /// match (`Chunk::check_sendable`).
    pub data_version: i32,
    /// `Data.spawn.pos`: the block a fresh connection spawns at, which is
    /// also the centre the chunk cache is pointed at (step 3.3e).
    ///
    /// SOURCE: wiki, unverified: the order inside `pos` is read as
    /// `[x, y, z]` (the field shape itself is the owner's template, the
    /// group of constants walked in `docs/SOURCES.md` step 3.0).
    pub spawn: Position,
}

/// The fields of `data/minecraft/world_gen_settings.dat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorldGenSettings {
    /// The `seed` the world was generated with; `hashed_seed` carries its
    /// hash into Login (play) (formula verified against the vanilla
    /// recording, parity test `login_fields`).
    pub seed: i64,
}

/// Reads `level.dat` from `path`.
///
/// A file that does not decompress or does not hold a field is named by
/// path and detail — neither the version nor the spawn is guessed or
/// defaulted: without a data version no chunk passes the ready rule, and a
/// spawn is required to point the player at the world.
pub fn read_level_dat(path: impl AsRef<Path>) -> Result<LevelDat, WorldError> {
    let display = path.as_ref().display().to_string();
    let root = read_world_document(path)?;
    let data_version =
        int_of(&root, "Data", "DataVersion").map_err(|detail| WorldError::WorldDocument {
            path: display.clone(),
            detail,
        })?;
    let spawn = spawn_of(&root).map_err(|detail| WorldError::WorldDocument {
        path: display,
        detail,
    })?;
    Ok(LevelDat {
        data_version,
        spawn,
    })
}

/// Reads `data/minecraft/world_gen_settings.dat` from `path` (the file the
/// world generation settings are saved beside the regions).
///
/// A world without a readable seed is not guessed at either: `hashed_seed`
/// would point the client at a different light/biome computation than the
/// chunks of the save hold (step 3.3e keeps `0` when there is no world).
///
/// The seed lives inside the `data` envelope of the template
/// (`{data:{seed,...}, DataVersion}`); a bare `seed` at the root is the
/// other documented shape and still reads ([`seed_of`]).
pub fn read_world_gen_settings(path: impl AsRef<Path>) -> Result<WorldGenSettings, WorldError> {
    let display = path.as_ref().display().to_string();
    let root = read_world_document(path)?;
    let seed = seed_of(&root).map_err(|detail| WorldError::WorldDocument {
        path: display,
        detail,
    })?;
    Ok(WorldGenSettings { seed })
}

/// The gzip disk NBT document under `path`, read and decoded.
///
/// Public so the audit test of the template (`tests/template_docs.rs`) can
/// open every saved `.dat` the game writes; the server itself reads the two
/// documents above through their own readers.
pub fn read_world_document(path: impl AsRef<Path>) -> Result<NbtTag, WorldError> {
    let path = path.as_ref();
    let display = path.display().to_string();
    let compressed = fs::read(path).map_err(|source| WorldError::WorldFile {
        path: display.clone(),
        source,
    })?;
    let mut document = Vec::new();
    GzDecoder::new(&compressed[..])
        .read_to_end(&mut document)
        .map_err(|source| WorldError::WorldFile {
            path: display.clone(),
            source,
        })?;
    decode_disk(&document).map_err(|detail| WorldError::WorldDocument {
        path: display,
        detail,
    })
}

/// An `int` child of one compound child of the document, as a detail string
/// when the shape is different (the caller wraps it into its own error).
fn int_of(root: &NbtTag, field: &str, key: &str) -> Result<i32, String> {
    let NbtTag::Compound(root) = root else {
        return Err("root is not a compound".to_owned());
    };
    let Some(NbtTag::Compound(cell)) = root.get(field) else {
        return Err(format!("no {field} compound"));
    };
    match cell.get(key) {
        Some(NbtTag::Int(value)) => Ok(*value),
        _ => Err(format!("no {key} int in {field}")),
    }
}

/// The `seed` of the generation document as a detail string otherwise.
///
/// The game wraps these documents in a `data` compound — the template's
/// bytes hold `{data:{bonus_chest, seed, generate_structures,
/// dimensions}, DataVersion}` — so `data.seed` is read first and a bare
/// root `seed` is the fallback for the other writing.
fn seed_of(root: &NbtTag) -> Result<i64, String> {
    let NbtTag::Compound(root) = root else {
        return Err("root is not a compound".to_owned());
    };
    if let Some(NbtTag::Compound(data)) = root.get("data") {
        if let Some(NbtTag::Long(seed)) = data.get("seed") {
            return Ok(*seed);
        }
    }
    match root.get("seed") {
        Some(NbtTag::Long(value)) => Ok(*value),
        _ => Err("no seed long under data or at the root".to_owned()),
    }
}

/// `Data.spawn.pos` of the document as the spawn block, as a detail string
/// otherwise (exactly three coordinates — a shorter or longer array is not
/// a position we can teleport anybody to).
fn spawn_of(root: &NbtTag) -> Result<Position, String> {
    let NbtTag::Compound(root) = root else {
        return Err("root is not a compound".to_owned());
    };
    let Some(NbtTag::Compound(data)) = root.get("Data") else {
        return Err("no Data compound".to_owned());
    };
    let Some(NbtTag::Compound(spawn)) = data.get("spawn") else {
        return Err("no spawn compound in Data".to_owned());
    };
    match spawn.get("pos") {
        Some(NbtTag::IntArray(pos)) => match pos.as_slice() {
            [x, y, z] => Ok(Position {
                x: *x,
                y: *y,
                z: *z,
            }),
            other => Err(format!("spawn.pos holds {} coordinates", other.len())),
        },
        _ => Err("no pos int array in Data.spawn".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// The document's inner NBT (before gzip): an empty root name, then the
    /// `Data` compound with `DataVersion` and the `spawn` compound inside.
    fn level_nbt(data_version: i32) -> Vec<u8> {
        let mut out = vec![0x0A, 0x00, 0x00]; // compound, empty root name
        out.extend_from_slice(&[0x0A, 0x00, 0x04]); // compound named `Data`
        out.extend_from_slice(b"Data");
        out.extend_from_slice(&[0x03, 0x00, 0x0B]); // int named `DataVersion`
        out.extend_from_slice(b"DataVersion");
        out.extend_from_slice(&data_version.to_be_bytes());
        out.extend_from_slice(&[0x0A, 0x00, 0x05]); // compound named `spawn`
        out.extend_from_slice(b"spawn");
        out.extend_from_slice(&[0x0B, 0x00, 0x03]); // int array named `pos`
        out.extend_from_slice(b"pos");
        out.extend_from_slice(&3_i32.to_be_bytes()); // three coordinates
        out.extend_from_slice(&96_i32.to_be_bytes()); // x
        out.extend_from_slice(&145_i32.to_be_bytes()); // y
        out.extend_from_slice(&(-32_i32).to_be_bytes()); // z
        out.push(0x00); // end of spawn
        out.push(0x00); // end of Data
        out.push(0x00); // end of the root
        out
    }

    /// The world generation document in the shape of the real template
    /// (walked byte by byte): the root holds the `data` compound — with
    /// `bonus_chest`, the `seed` long, `generate_structures` and the
    /// `dimensions` map — and the `DataVersion` int beside it. Stream
    /// order matches the file: `data` first, `DataVersion` last.
    fn settings_nbt(seed: i64) -> Vec<u8> {
        let mut out = vec![0x0A, 0x00, 0x00]; // compound, empty root name
        out.extend_from_slice(&[0x0A, 0x00, 0x04]); // compound named `data`
        out.extend_from_slice(b"data");
        out.extend_from_slice(&[0x01, 0x00, 0x0B]); // byte named `bonus_chest`
        out.extend_from_slice(b"bonus_chest");
        out.push(0x00); // false
        out.extend_from_slice(&[0x04, 0x00, 0x04]); // long named `seed`
        out.extend_from_slice(b"seed");
        out.extend_from_slice(&seed.to_be_bytes());
        out.extend_from_slice(&[0x01, 0x00, 0x13]); // byte `generate_structures`
        out.extend_from_slice(b"generate_structures");
        out.push(0x01); // true
        out.extend_from_slice(&[0x0A, 0x00, 0x0A]); // compound `dimensions`
        out.extend_from_slice(b"dimensions");
        out.push(0x00); // empty map: dimensions are not read here
        out.push(0x00); // end of data
        out.extend_from_slice(&[0x03, 0x00, 0x0B]); // int named `DataVersion`
        out.extend_from_slice(b"DataVersion");
        out.extend_from_slice(&5023_i32.to_be_bytes());
        out.push(0x00); // end of the root
        out
    }

    /// The other documented shape: `seed` lying directly at the root (the
    /// fallback the reader must keep honouring).
    fn flat_settings_nbt(seed: i64) -> Vec<u8> {
        let mut out = vec![0x0A, 0x00, 0x00]; // compound, empty root name
        out.extend_from_slice(&[0x04, 0x00, 0x04]); // long named `seed`
        out.extend_from_slice(b"seed");
        out.extend_from_slice(&seed.to_be_bytes());
        out.push(0x00); // end of the root
        out
    }

    /// Writes a gzip NBT document to a fresh temp file.
    fn gzip_file(name: &str, document: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("mine_rust-{name}.dat"));
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(document).expect("gzip");
        let bytes = encoder.finish().expect("finished");
        fs::write(&path, bytes).expect("written");
        path
    }

    #[test]
    fn version_and_spawn_come_out_of_the_gzipped_document() {
        let path = gzip_file("level-ok", &level_nbt(5023));
        let level = read_level_dat(&path).expect("valid document");
        assert_eq!(level.data_version, 5023, "as stored in the template");
        assert_eq!(
            level.spawn,
            Position {
                x: 96,
                y: 145,
                z: -32
            },
            "the three coordinates of spawn.pos, in order"
        );
        let _ = fs::remove_file(&path);
    }

    /// The shape of the owner's template: `seed` inside `data`. This is the
    /// regression test of the step 3.3 live check — the flat document used
    /// to be the only shape the reader accepted.
    #[test]
    fn the_seed_comes_out_of_the_data_envelope_of_the_document() {
        let path = gzip_file("settings-ok", &settings_nbt(12345));
        let settings = read_world_gen_settings(&path).expect("valid document");
        assert_eq!(settings.seed, 12345, "as stored by the game");
        let _ = fs::remove_file(&path);
    }

    /// The fallback shape (a bare root `seed`) must keep reading.
    #[test]
    fn a_seed_at_the_root_of_the_document_still_reads() {
        let path = gzip_file("settings-flat", &flat_settings_nbt(777));
        let settings = read_world_gen_settings(&path).expect("valid document");
        assert_eq!(settings.seed, 777, "the other documented shape");
        let _ = fs::remove_file(&path);
    }

    /// Neither shape present: the field is named, never defaulted.
    #[test]
    fn a_generation_document_without_any_seed_is_refused() {
        let mut document = settings_nbt(12345);
        document.clear();
        document.extend_from_slice(&[0x0A, 0x00, 0x00]); // empty root compound
        document.push(0x00);
        let path = gzip_file("settings-noseed", &document);
        let error = read_world_gen_settings(&path).expect_err("no seed at all");
        assert!(
            matches!(&error, WorldError::WorldDocument { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("no seed"), "{error}");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_document_without_the_field_is_named_not_defaulted() {
        let path = gzip_file("level-missing", b"");
        let error = read_level_dat(&path).expect_err("no root document");
        assert!(
            matches!(&error, WorldError::WorldDocument { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("NBT document"), "{error}");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_level_without_a_spawn_is_refused_with_the_missing_field() {
        let path = gzip_file("level-nospawn", &settings_nbt(1)); // not level.dat
        let error = read_level_dat(&path).expect_err("no Data.spawn");
        assert!(
            matches!(&error, WorldError::WorldDocument { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("Data"), "{error}");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_file_that_is_not_gzip_is_a_read_error_of_that_path() {
        let path = std::env::temp_dir().join("mine_rust-level-plain.dat");
        fs::write(&path, level_nbt(1)).expect("raw NBT without gzip");
        let error = read_level_dat(&path).expect_err("gzip required");
        assert!(matches!(&error, WorldError::WorldFile { .. }), "{error}");
        assert!(error.to_string().contains("level-plain.dat"), "{error}");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn an_absent_file_names_the_path() {
        let error = read_level_dat("definitely-not-here.dat").expect_err("missing");
        assert!(matches!(&error, WorldError::WorldFile { .. }), "{error}");
        assert!(
            error.to_string().contains("definitely-not-here.dat"),
            "{error}"
        );
    }
}
