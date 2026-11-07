//! Numeric entry ids of the built-in registries from the data generator's
//! registries report.
//!
//! The data generator (`--reports`) writes `reports/registries.json`: for
//! every registry it knows, for every entry, the numeric `protocol_id` the
//! protocol uses. The registries of the Update Tags packet that the server
//! does **not** synchronize in the Registry Data packet (`block`, `item`,
//! `fluid`, `potion`, ...) refer to those static ids — they are exactly the
//! ids the client already knows from its own build.
//!
//! Like every other report the file stays outside the repository and is
//! pointed at by the optional `registries_report` setting of the server
//! configuration. Synchronized registries do not appear in the report; their
//! ids come from the positions of the Registry Data packet instead.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::error::DataError;

/// Numeric id of one registry entry of the report.
#[derive(Debug, Deserialize)]
struct ProtocolId {
    /// Id as it appears in the report.
    protocol_id: i32,
}

/// One registry of the report: its entries with their protocol ids.
#[derive(Debug, Deserialize)]
struct RegistryReport {
    /// Entries of the registry by identifier.
    entries: BTreeMap<String, ProtocolId>,
}

/// Numeric entry ids of every registry of `reports/registries.json`,
/// keyed by registry identifier (`minecraft:block` and friends).
#[derive(Debug, Default, Clone)]
pub struct RegistryIds {
    /// Registry id → entry identifier → numeric protocol id.
    registries: BTreeMap<String, BTreeMap<String, i32>>,
}

impl RegistryIds {
    /// Number of registries the report describes.
    pub fn len(&self) -> usize {
        self.registries.len()
    }

    /// Whether the report describes no registry at all (the default). A
    /// report that is read but empty is rejected by the parser.
    pub fn is_empty(&self) -> bool {
        self.registries.is_empty()
    }

    /// Identifiers of the registries the report describes.
    pub fn registry_ids(&self) -> impl Iterator<Item = &str> {
        self.registries.keys().map(String::as_str)
    }

    /// Entry ids of one registry (`minecraft:block`), if the report
    /// describes it.
    pub(crate) fn get(&self, registry_id: &str) -> Option<&BTreeMap<String, i32>> {
        self.registries.get(registry_id)
    }
}

/// Loads `reports/registries.json` from `path` (the data generator's report).
pub fn load_registries_report(path: impl AsRef<Path>) -> Result<RegistryIds, DataError> {
    let path_ref = path.as_ref();
    let display = path_ref.display().to_string();
    let text = fs::read_to_string(path_ref).map_err(|source| DataError::RegistriesReportRead {
        path: display.clone(),
        source,
    })?;
    registries_from_json_named(&text, display)
}

/// Parses the JSON text of a registries report.
pub fn registries_from_json(text: &str) -> Result<RegistryIds, DataError> {
    registries_from_json_named(text, "registries report".to_owned())
}

/// Shared parser: turns the report text into ids, naming `path` in errors.
fn registries_from_json_named(text: &str, path: String) -> Result<RegistryIds, DataError> {
    let file: BTreeMap<String, RegistryReport> =
        serde_json::from_str(text).map_err(|source| DataError::Json {
            path: path.clone(),
            source,
        })?;
    if file.is_empty() {
        return Err(DataError::EmptyReport { path });
    }
    Ok(RegistryIds {
        registries: file
            .into_iter()
            .map(|(registry, report)| {
                let entries = report
                    .entries
                    .into_iter()
                    .map(|(entry, id)| (entry, id.protocol_id))
                    .collect();
                (registry, entries)
            })
            .collect(),
    })
}
