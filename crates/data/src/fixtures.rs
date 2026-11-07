//! In-memory fixture jars for tests (feature `fixtures`).
//!
//! Test jars are built from file lists defined by each test itself; no game
//! data is stored in the repository. Normal builds never enable this feature.

use std::io::{Cursor, Write};

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::load::JAR_DATA_PREFIX;
use crate::SYNCHRONIZED_REGISTRIES;
use protocol::{used_packet_keys, Direction, PacketKey, ProtocolState};

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

/// Builds the JSON text of a packets report (`reports/packets.json`) from
/// [`used_packet_keys`], applying `overrides` (key → id).
///
/// Every key of the server appears with its compiled default id unless
/// overridden, so a test can prove that the server sends the id taken from
/// the report and not the compiled default. No game data is stored in the
/// repository; the fixture duplicates only the shape the parser expects.
pub fn packets_report_json(overrides: &[(PacketKey, u32)]) -> String {
    use std::collections::BTreeMap;

    let states = [
        ProtocolState::Handshake,
        ProtocolState::Status,
        ProtocolState::Login,
        ProtocolState::Configuration,
        ProtocolState::Play,
    ];
    let mut text = String::from("{");
    for (index, state) in states.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        text.push_str(&format!("\"{}\":{{", state.as_str()));
        for (direction_index, direction) in [Direction::Clientbound, Direction::Serverbound]
            .into_iter()
            .enumerate()
        {
            if direction_index > 0 {
                text.push(',');
            }
            // BTreeMap keeps the entry order stable between test runs.
            let entries: BTreeMap<&str, u32> = used_packet_keys()
                .iter()
                .filter(|key| key.state == *state && key.direction == direction)
                .map(|key| {
                    let id = overrides
                        .iter()
                        .find(|(overridden, _)| *overridden == *key)
                        .map_or(key.default_id, |(_, id)| *id);
                    (key.name, id)
                })
                .collect();
            text.push_str(&format!("\"{}\":{{", direction.as_str()));
            for (position, (name, id)) in entries.iter().enumerate() {
                if position > 0 {
                    text.push(',');
                }
                text.push_str(&format!("\"{name}\":{{\"protocol_id\":{id}}}"));
            }
            text.push('}');
        }
        text.push('}');
    }
    text.push('}');
    text
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
