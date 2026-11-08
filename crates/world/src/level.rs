//! Reading `level.dat`: the world document saved next to the region files.
//!
//! SOURCE: the owner's template — the real file was walked byte by byte:
//! a gzip container around the disk NBT form of [`crate::nbt::decode_disk`],
//! whose root compound holds `Data` with the `DataVersion` integer the send
//! rule compares every chunk against (`docs/PROGRESS.md`, template rule).
//! The spawn of the same compound is taken from here in step 3.3e.

use std::fs;
use std::io::Read;
use std::path::Path;

use flate2::read::GzDecoder;
use protocol::NbtTag;

use crate::error::WorldError;
use crate::nbt::decode_disk;

/// The fields of `level.dat` the server has a use for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelDat {
    /// `Data.DataVersion`: the schema every sent chunk of the save must
    /// match (`Chunk::check_sendable`).
    pub data_version: i32,
}

/// Reads `level.dat` from `path` and returns the world's data version.
///
/// A file that does not decompress or does not hold the field is named by
/// path and detail — the version is never guessed or defaulted: without it
/// no chunk passes the ready rule, which refuses everything by itself.
pub fn read_level_dat(path: impl AsRef<Path>) -> Result<LevelDat, WorldError> {
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
    let root = decode_disk(&document).map_err(|detail| WorldError::WorldDocument {
        path: display.clone(),
        detail,
    })?;
    let data_version =
        int_of(&root, "Data", "DataVersion").map_err(|detail| WorldError::WorldDocument {
            path: display,
            detail,
        })?;
    Ok(LevelDat { data_version })
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

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    /// The document's inner NBT (before gzip): an empty root name, then the
    /// `Data` compound with the `DataVersion` int inside.
    fn level_nbt(data_version: i32) -> Vec<u8> {
        let mut out = vec![0x0A, 0x00, 0x00]; // compound, empty root name
        out.extend_from_slice(&[0x0A, 0x00, 0x04]); // compound named `Data`
        out.extend_from_slice(b"Data");
        out.extend_from_slice(&[0x03, 0x00, 0x0B]); // int named `DataVersion`
        out.extend_from_slice(b"DataVersion");
        out.extend_from_slice(&data_version.to_be_bytes());
        out.push(0x00); // end of Data
        out.push(0x00); // end of the root
        out
    }

    /// Writes the gzip NBT of one data version to a fresh temp file.
    fn level_file(name: &str, data_version: i32) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("mine_rust-level-{name}.dat"));
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&level_nbt(data_version)).expect("gzip");
        let bytes = encoder.finish().expect("finished");
        fs::write(&path, bytes).expect("written");
        path
    }

    #[test]
    fn the_data_version_comes_out_of_the_gzipped_document() {
        let path = level_file("ok", 5023);
        let level = read_level_dat(&path).expect("valid document");
        assert_eq!(level.data_version, 5023, "as stored in the template");
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn a_document_without_the_field_is_named_not_defaulted() {
        let path = std::env::temp_dir().join("mine_rust-level-missing.dat");
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"").expect("gzip");
        let bytes = encoder.finish().expect("finished");
        fs::write(&path, bytes).expect("written");
        let error = read_level_dat(&path).expect_err("no root document");
        assert!(
            matches!(&error, WorldError::WorldDocument { .. }),
            "{error}"
        );
        assert!(error.to_string().contains("NBT document"), "{error}");
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
