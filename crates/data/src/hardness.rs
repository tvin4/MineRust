//! Block hardness per state, read from `reports/hardness.json`.
//!
//! The server-side mining timer (Шаг 16a) must know how hard a block is before
//! it can estimate how long a dig lasts. The data generator's
//! `reports/blocks.json` lists state ids and properties but no hardness, so the
//! test client's `hardness` command measures
//! `BlockState.getDestroySpeed(level, pos)` (the block hardness; negative means
//! unbreakable) and `Player.getDestroySpeed(state)` (the bare-hand dig speed)
//! for every state through public game API and writes this report.
//!
//! The file is generated from the user's own copy of the game and stays outside
//! the repository, like every other report; the server points at it with the
//! optional `hardness_report` setting.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::error::DataError;

/// Hardness and bare-hand dig speed of every block state, indexed by the
/// numeric state id of `reports/blocks.json`.
#[derive(Debug, Default, Clone)]
pub struct BlockHardness {
    /// State id → block hardness (destroy speed). A negative value means the
    /// state cannot be broken at all.
    hardness: BTreeMap<i32, f64>,
    /// State id → dig speed of a player holding nothing. Measured with an
    /// empty hand; the server uses it as the dig speed of a bare-handed dig
    /// until inventories exist.
    hand_speed: BTreeMap<i32, f64>,
    /// States that require the right tool for drops. A bare hand is not a
    /// right tool, so the game divides the dig progress by 100 instead of 30
    /// (Шаг 17a). A report without the list leaves every state unflagged.
    tool_required: BTreeSet<i32>,
}

impl BlockHardness {
    /// Reads the report from `path`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, DataError> {
        let path_ref = path.as_ref();
        let display = path_ref.display().to_string();
        let text =
            fs::read_to_string(path_ref).map_err(|source| DataError::HardnessReportRead {
                path: display.clone(),
                source,
            })?;
        Self::parse(&text, display)
    }

    /// Parses the JSON text of a hardness report, naming `path` in errors.
    ///
    /// Only the `hardness` map is required; `handSpeed` is optional (a report
    /// without it leaves every speed absent, and the caller falls back to a
    /// bare-hand speed of `1.0`), and `toolRequired` is optional (a report
    /// without it leaves every state unflagged, i.e. the divisor of 30).
    pub fn parse(text: &str, path: String) -> Result<Self, DataError> {
        let parsed: Value = serde_json::from_str(text).map_err(|source| DataError::Json {
            path: path.clone(),
            source,
        })?;
        let Some(hardness) = parsed.get("hardness").and_then(Value::as_object) else {
            return Err(DataError::EmptyHardnessReport { path });
        };
        if hardness.is_empty() {
            return Err(DataError::EmptyHardnessReport { path });
        }
        let mut speeds = BTreeMap::new();
        let mut hardnesses = BTreeMap::new();
        let mut tools = BTreeSet::new();
        for (key, value) in hardness {
            let Some(id) = key.parse::<i32>().ok() else {
                continue;
            };
            if let Some(value) = value.as_f64() {
                hardnesses.insert(id, value);
            }
        }
        if let Some(hand_speed) = parsed.get("handSpeed").and_then(Value::as_object) {
            for (key, value) in hand_speed {
                let Some(id) = key.parse::<i32>().ok() else {
                    continue;
                };
                if let Some(value) = value.as_f64() {
                    speeds.insert(id, value);
                }
            }
        }
        if let Some(tool_required) = parsed.get("toolRequired").and_then(Value::as_array) {
            for value in tool_required {
                if let Some(id) = value.as_i64().and_then(|id| i32::try_from(id).ok()) {
                    tools.insert(id);
                }
            }
        }
        Ok(Self {
            hardness: hardnesses,
            hand_speed: speeds,
            tool_required: tools,
        })
    }

    /// Hardness (destroy speed) of the state, or `None` when the report does
    /// not describe it. A negative value means unbreakable.
    pub fn hardness(&self, state_id: i32) -> Option<f64> {
        self.hardness.get(&state_id).copied()
    }

    /// Bare-hand dig speed of the state, or `None` when the report does not
    /// carry one (the caller then assumes `1.0`).
    pub fn hand_speed(&self, state_id: i32) -> Option<f64> {
        self.hand_speed.get(&state_id).copied()
    }

    /// Whether the state is reported as unbreakable (a negative hardness).
    /// A state absent from the report is **not** treated as unbreakable.
    pub fn is_unbreakable(&self, state_id: i32) -> bool {
        self.hardness(state_id).is_some_and(|value| value < 0.0)
    }

    /// Whether the state requires the right tool for drops. A bare hand is
    /// never that tool, so such a dig advances at `/100` instead of `/30`
    /// (Шаг 17a). A state absent from the list does **not** require a tool.
    pub fn requires_tool(&self, state_id: i32) -> bool {
        self.tool_required.contains(&state_id)
    }

    /// Number of states flagged as requiring the right tool.
    pub fn tool_required_len(&self) -> usize {
        self.tool_required.len()
    }

    /// Number of states the report describes.
    pub fn len(&self) -> usize {
        self.hardness.len()
    }

    /// Whether the report describes no state at all.
    pub fn is_empty(&self) -> bool {
        self.hardness.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of the report the test client writes: the header fields, the
    /// hardness map, the bare-hand speeds and the unbreakable list.
    fn report() -> &'static str {
        r#"{
          "source": "BlockState.getDestroySpeed(level,pos) and Player.getDestroySpeed(state)",
          "states": 4,
          "hardness": {"1": 1.5, "10": 0.5, "0": 0.0, "34": -1.0},
          "handSpeed": {"1": 1.0, "10": 1.0, "0": 1.0, "34": 1.0},
          "unbreakable": [34],
          "toolRequired": [1]
        }"#
    }

    #[test]
    fn hardness_and_bare_hand_speed_are_read_per_state() {
        let report = BlockHardness::parse(report(), "test".to_owned()).expect("valid report");
        assert_eq!(report.hardness(1), Some(1.5), "stone");
        assert_eq!(report.hardness(10), Some(0.5), "dirt");
        assert_eq!(report.hardness(0), Some(0.0), "air is instant");
        assert_eq!(report.hand_speed(1), Some(1.0));
        assert_eq!(report.len(), 4);
        assert!(!report.is_empty());
    }

    #[test]
    fn a_negative_hardness_marks_the_state_unbreakable() {
        let report = BlockHardness::parse(report(), "test".to_owned()).expect("valid report");
        assert!(report.is_unbreakable(34), "hardness -1 is unbreakable");
        assert!(!report.is_unbreakable(1), "stone is breakable");
        assert!(
            !report.is_unbreakable(9999),
            "a state the report does not describe is not unbreakable"
        );
    }

    #[test]
    fn a_report_without_hand_speed_keeps_the_hardness() {
        let text = r#"{"states": 1, "hardness": {"1": 2.0}}"#;
        let report = BlockHardness::parse(text, "test".to_owned()).expect("valid report");
        assert_eq!(report.hardness(1), Some(2.0));
        assert_eq!(report.hand_speed(1), None, "no speed in this report");
    }

    /// Шаг 17a: the tool flag is a list of state ids; a report without the
    /// list leaves every state unflagged, so old reports keep loading.
    #[test]
    fn the_tool_flag_is_read_per_state_and_defaults_to_false() {
        let report = BlockHardness::parse(report(), "test".to_owned()).expect("valid report");
        assert!(report.requires_tool(1), "stone needs a pickaxe");
        assert!(!report.requires_tool(10), "dirt does not need a tool");
        assert!(
            !report.requires_tool(9999),
            "a state outside the list needs none"
        );
        assert_eq!(report.tool_required_len(), 1);

        let old = BlockHardness::parse(
            r#"{"states": 1, "hardness": {"1": 1.5}}"#,
            "test".to_owned(),
        )
        .expect("a report without the flag still loads");
        assert!(!old.requires_tool(1), "no list, no flagged state");
        assert_eq!(old.tool_required_len(), 0);
    }

    #[test]
    fn an_empty_hardness_map_is_rejected() {
        let error = BlockHardness::parse(r#"{"hardness": {}}"#, "test".to_owned())
            .expect_err("an empty report is not a hardness report");
        assert!(matches!(error, DataError::EmptyHardnessReport { .. }));
    }

    #[test]
    fn a_document_without_hardness_is_rejected() {
        let error = BlockHardness::parse(r#"{"states": 1}"#, "test".to_owned())
            .expect_err("the hardness field is required");
        assert!(matches!(error, DataError::EmptyHardnessReport { .. }));
    }
}
