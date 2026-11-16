//! Collision-shape bottoms per block state, read from `reports/collision.json`.
//!
//! The free-headroom query of the crawl rule (Шаг 15 п.5) needs to know where
//! the collision boxes of the block above a player start inside its cell. The
//! data generator's `reports/blocks.json` lists state ids and properties but no
//! shapes, so the test client's `shapes` command measures
//! `BlockState.getCollisionShape(level, pos).min(Y)` for every state through
//! public game API and writes this report. States with an empty collision shape
//! are omitted (they never obstruct a head).
//!
//! The file is generated from the user's own copy of the game and stays outside
//! the repository, like every other report; the server points at it with the
//! optional `collision_report` setting.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::error::DataError;

/// Bottom of the collision shape of every non-empty state, indexed by the
/// numeric state id of `reports/blocks.json`.
#[derive(Debug, Default, Clone)]
pub struct CollisionShapes {
    /// State id → lowest Y of the collision boxes relative to the cell base.
    bottoms: BTreeMap<i32, f64>,
}

impl CollisionShapes {
    /// Reads the report from `path`.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, DataError> {
        let path_ref = path.as_ref();
        let display = path_ref.display().to_string();
        let text =
            fs::read_to_string(path_ref).map_err(|source| DataError::CollisionReportRead {
                path: display.clone(),
                source,
            })?;
        Self::parse(&text, display)
    }

    /// Parses the JSON text of a collision report, naming `path` in errors.
    pub fn parse(text: &str, path: String) -> Result<Self, DataError> {
        let parsed: Value = serde_json::from_str(text).map_err(|source| DataError::Json {
            path: path.clone(),
            source,
        })?;
        let Some(shapes) = parsed.get("shapes").and_then(Value::as_object) else {
            return Err(DataError::EmptyCollisionReport { path });
        };
        if shapes.is_empty() {
            return Err(DataError::EmptyCollisionReport { path });
        }
        let mut bottoms = BTreeMap::new();
        for (key, value) in shapes {
            let Some(id) = key.parse::<i32>().ok() else {
                continue;
            };
            if let Some(bottom) = value.as_f64() {
                bottoms.insert(id, bottom);
            }
        }
        Ok(Self { bottoms })
    }

    /// Lowest Y of the state's collision boxes relative to the cell base, or
    /// `None` when the state has no collision shape (air, plants, water, ...).
    pub fn bottom(&self, state_id: i32) -> Option<f64> {
        self.bottoms.get(&state_id).copied()
    }

    /// Number of states the report describes.
    pub fn len(&self) -> usize {
        self.bottoms.len()
    }

    /// Whether the report describes no state at all.
    pub fn is_empty(&self) -> bool {
        self.bottoms.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of the report the test client writes: the header fields plus
    /// a state-id keyed map, with air omitted.
    fn report() -> &'static str {
        r#"{
          "source": "BlockState.getCollisionShape(...).min(Y)",
          "states": 4,
          "shapes": {"1": 0.0, "15250": 0.5, "8639": 0.0}
        }"#
    }

    #[test]
    fn bottoms_are_read_per_state_and_air_is_absent() {
        let shapes = CollisionShapes::parse(report(), "test".to_owned()).expect("valid report");
        assert_eq!(shapes.bottom(1), Some(0.0));
        assert_eq!(shapes.bottom(15250), Some(0.5));
        assert_eq!(shapes.bottom(8639), Some(0.0));
        assert_eq!(shapes.bottom(0), None, "air is not in the report");
        assert_eq!(shapes.len(), 3);
    }

    #[test]
    fn an_empty_shape_map_is_rejected_not_treated_as_clear() {
        let error = CollisionShapes::parse(r#"{"shapes": {}}"#, "test".to_owned())
            .expect_err("an empty report is not a valid collision report");
        assert!(matches!(error, DataError::EmptyCollisionReport { .. }));
    }

    #[test]
    fn a_document_without_shapes_is_rejected() {
        let error = CollisionShapes::parse(r#"{"states": 1}"#, "test".to_owned())
            .expect_err("the shapes field is required");
        assert!(matches!(error, DataError::EmptyCollisionReport { .. }));
    }
}
