//! Block state ids from the data generator's `reports/blocks.json`.
//!
//! The report is generated from the user's own copy of the game and lists,
//! per block, every state with its numeric id and the property values that
//! state accepts (one entry also carries `"default": true`). The stored
//! chunk palettes hold block names plus properties; the wire palette holds
//! these numeric state ids — so this file is what turns one into the other.
//!
//! Like every other report the file stays outside the repository and is
//! pointed at by the optional `blocks_report` setting of the server
//! configuration.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::error::DataError;

/// Property values of one state, as written in the report
/// (`{"facing": "north"}` → `{"facing": "north"}`).
pub type Properties = BTreeMap<String, String>;

/// State ids of every block of the game copy, read from `reports/blocks.json`.
#[derive(Debug, Default, Clone)]
pub struct BlockStates {
    /// Block name → id of its default state.
    default: BTreeMap<String, i32>,
    /// State id → block name of that state.
    names: BTreeMap<i32, String>,
    /// Block name → every state of that block (id + accepted values).
    states: BTreeMap<String, Vec<(i32, Properties)>>,
}

impl BlockStates {
    /// Reads the report from `path`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, DataError> {
        let path_ref = path.as_ref();
        let display = path_ref.display().to_string();
        let text = fs::read_to_string(path_ref).map_err(|source| DataError::BlocksReportRead {
            path: display.clone(),
            source,
        })?;
        Self::parse(&text, display)
    }

    /// Parses the JSON text of a blocks report, naming `path` in errors.
    pub fn parse(text: &str, path: String) -> Result<Self, DataError> {
        let parsed: BTreeMap<String, Value> =
            serde_json::from_str(text).map_err(|source| DataError::Json {
                path: path.clone(),
                source,
            })?;
        if parsed.is_empty() {
            return Err(DataError::EmptyBlocksReport { path });
        }
        Ok(Self::from_json(parsed))
    }

    /// Builds the lookup tables from an already parsed report.
    fn from_json(parsed: BTreeMap<String, Value>) -> Self {
        let mut report = Self::default();
        for (name, block) in parsed {
            let Some(states) = block["states"].as_array() else {
                continue;
            };
            let mut all = Vec::new();
            for state in states {
                let Some(id) = state["id"].as_i64() else {
                    continue;
                };
                let id = id as i32;
                report.names.entry(id).or_insert_with(|| name.clone());
                if state["default"] == Value::Bool(true) {
                    report.default.insert(name.clone(), id);
                }
                all.push((id, property_values(state)));
            }
            if let Some(first) = all.first().map(|(id, _)| *id) {
                report.default.entry(name.clone()).or_insert(first);
            }
            report.states.insert(name, all);
        }
        report
    }

    /// State id of the block's default state, if the report lists the block.
    pub fn default_id(&self, name: &str) -> Option<i32> {
        self.default.get(name).copied()
    }

    /// State id a stored palette entry stands for: the block default for an
    /// entry without properties, or the id of the state whose accepted values
    /// match every requested property (`None` when no state matches).
    pub fn state_id(&self, name: &str, properties: Option<&Properties>) -> Option<i32> {
        let Some(requested) = properties else {
            return self.default_id(name);
        };
        let candidates = self.states.get(name)?;
        candidates
            .iter()
            .find(|(_, values)| values == requested)
            .map(|(id, _)| *id)
    }

    /// Block name of a state id, if the report knows the id.
    pub fn block_name(&self, state_id: i32) -> Option<&str> {
        self.names.get(&state_id).map(String::as_str)
    }

    /// Number of blocks the report describes.
    pub fn len(&self) -> usize {
        self.states.len()
    }

    /// Whether the report describes no block at all.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }
}

/// The property values of one state record.
fn property_values(state: &Value) -> Properties {
    state["properties"]
        .as_object()
        .map(|object| {
            object
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slice of the report shape: two blocks, one with a default marker and
    /// one whose default is simply the first state.
    fn report() -> &'static str {
        r#"{
          "minecraft:air": {"states": [{"id": 0, "default": true}]},
          "minecraft:oak_slab": {"states": [
            {"id": 41, "default": true},
            {"id": 42, "default": false, "properties": {"type": "top"}},
            {"id": 43, "default": false, "properties": {"type": "bottom"}}
          ]}
        }"#
    }

    #[test]
    fn defaults_come_from_the_marker_or_the_first_state() {
        let states = BlockStates::parse(report(), "test".to_owned()).expect("valid report");
        assert_eq!(states.default_id("minecraft:air"), Some(0));
        assert_eq!(states.default_id("minecraft:oak_slab"), Some(41));
        assert_eq!(states.default_id("minecraft:stone"), None);
        assert_eq!(states.len(), 2);
        assert!(!states.is_empty());
    }

    #[test]
    fn a_state_resolves_through_all_its_properties() {
        let states = BlockStates::parse(report(), "test".to_owned()).expect("valid report");
        let top = Properties::from([("type".to_owned(), "top".to_owned())]);
        assert_eq!(states.state_id("minecraft:oak_slab", Some(&top)), Some(42));
        // An entry without properties is the block default.
        assert_eq!(states.state_id("minecraft:oak_slab", None), Some(41));
        // A property no state accepts resolves to nothing, not to a guess.
        let middle = Properties::from([("type".to_owned(), "middle".to_owned())]);
        assert_eq!(states.state_id("minecraft:oak_slab", Some(&middle)), None);
    }

    #[test]
    fn every_state_id_resolves_back_to_its_block() {
        let states = BlockStates::parse(report(), "test".to_owned()).expect("valid report");
        assert_eq!(states.block_name(43), Some("minecraft:oak_slab"));
        assert_eq!(states.block_name(999), None);
    }

    #[test]
    fn a_broken_report_names_the_file() {
        let error =
            BlockStates::parse("{", "reports/blocks.json".to_owned()).expect_err("not JSON");
        assert!(matches!(error, DataError::Json { .. }), "{error}");
        let empty =
            BlockStates::parse("{}", "reports/blocks.json".to_owned()).expect_err("no blocks");
        assert!(
            matches!(empty, DataError::EmptyBlocksReport { .. }),
            "{empty}"
        );
    }

    #[test]
    fn a_missing_report_file_is_reported() {
        let error = BlockStates::load("definitely-not-here.json").expect_err("missing");
        assert!(
            matches!(error, DataError::BlocksReportRead { .. }),
            "{error}"
        );
    }
}
