//! In-memory fixture jars for tests (feature `fixtures`).
//!
//! Test jars are built from file lists defined by each test itself; no game
//! data is stored in the repository. Normal builds never enable this feature.

use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::load::JAR_DATA_PREFIX;
use crate::SYNCHRONIZED_REGISTRIES;

/// Builds an in-memory jar from `files`, plus a placeholder entry for every
/// synchronized registry the caller does not fill itself (each registry must
/// be non-empty for the loader).
pub fn jar_with(files: &[(&str, &str)]) -> Cursor<Vec<u8>> {
    let mut all: Vec<(String, String)> = files
        .iter()
        .map(|(name, content)| ((*name).to_owned(), (*content).to_owned()))
        .collect();
    for dir in SYNCHRONIZED_REGISTRIES {
        let prefix = format!("{JAR_DATA_PREFIX}{dir}/");
        if !all.iter().any(|(name, _)| name.starts_with(&prefix)) {
            all.push((format!("{prefix}fixture.json"), "{}".to_owned()));
        }
    }
    write_jar(&all)
}

/// Builds an in-memory jar with exactly the given files and no placeholders.
pub fn jar_exactly(files: &[(&str, &str)]) -> Cursor<Vec<u8>> {
    let owned: Vec<(String, String)> = files
        .iter()
        .map(|(name, content)| ((*name).to_owned(), (*content).to_owned()))
        .collect();
    write_jar(&owned)
}

/// Writes `files` into an in-memory zip stream.
fn write_jar(files: &[(String, String)]) -> Cursor<Vec<u8>> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in files {
        writer
            .start_file(name.as_str(), SimpleFileOptions::default())
            .expect("fixture entry");
        writer
            .write_all(content.as_bytes())
            .expect("fixture content");
    }
    writer.finish().expect("fixture jar")
}
