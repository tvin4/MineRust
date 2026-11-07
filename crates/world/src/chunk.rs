//! The stored chunk model and the data-driven rule for sending one.

use std::collections::BTreeMap;

use protocol::NbtTag;

use crate::error::WorldError;
use crate::region::StoredChunk;

/// Root `Status` values of a chunk that may go on the wire.
///
/// Both are read from the owner's template against the vanilla join trace
/// (step 3.0): in the very same world vanilla sent exactly the nine chunks
/// whose `Status` is one of these two, and those are also the only chunks
/// whose sections carry light arrays — the rule is the intersection of the
/// observations, not a guess. Every other `Status` in the template means the
/// chunk still waits for the generator (M6).
pub const READY_STATUSES: [&str; 2] = ["minecraft:full", "minecraft:initialize_light"];

/// Why a stored chunk is refused with a name instead of an error string.
///
/// Every variant names data that was actually read from the file: the rule
/// never invents a reason.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ChunkNotReady {
    /// The chunk is stored under a different schema than the world being served.
    #[error("chunk ({x}, {z}) is saved with DataVersion {found}, expected {expected}")]
    UnsupportedVersion {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
        /// DataVersion found in the chunk NBT.
        found: i32,
        /// DataVersion of the world being served.
        expected: i32,
    },
    /// The generation status of the chunk is a stage before the two ready ones.
    #[error("chunk ({x}, {z}) is not generated: Status = {status}")]
    NotGenerated {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
        /// Root `Status` as stored.
        status: String,
    },
    /// The status is ready but no section carries any light array.
    #[error("chunk ({x}, {z}) has no BlockLight/SkyLight in its sections")]
    NoLight {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
    },
}

/// One section of a chunk as stored: its palette, packed data and light.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// Section Y from −4 to 19 (stored as a byte).
    pub y: i8,
    /// Full `block_states` compound (`palette` + packed `data`), untouched.
    pub block_states: Option<NbtTag>,
    /// Full `biomes` compound (`palette` + packed `data`), untouched.
    pub biomes: Option<NbtTag>,
    /// Block light bytes as stored, `None` when the section carries none.
    pub block_light: Option<Vec<i8>>,
    /// Sky light bytes as stored, `None` when the section carries none.
    pub sky_light: Option<Vec<i8>>,
}

/// A chunk of the world, read from a region file.
///
/// Fields this model does not interpret (`Heightmaps`, `PostProcessing`,
/// ticks, entities, structures, ...) stay available in [`Chunk::root`] so a
/// later step can widen the model without losing information.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// Chunk x.
    pub x: i32,
    /// Chunk z.
    pub z: i32,
    /// `DataVersion` of the schema this chunk was saved with.
    pub data_version: i32,
    /// Root `Status`, e.g. `minecraft:full`.
    pub status: String,
    /// The twenty-four observed sections, bottom to top.
    pub sections: Vec<Section>,
    /// The untouched root compound of the stored chunk.
    pub root: NbtTag,
}

impl Chunk {
    /// Turns one stored chunk into the model, rejecting anything the reader
    /// does not recognise (a wrong root type or a missing/wrongly typed
    /// field is an explicit error, never a default).
    pub fn from_stored(stored: &StoredChunk) -> Result<Self, WorldError> {
        let (x, z) = (stored.x, stored.z);
        let root = match &stored.nbt {
            NbtTag::Compound(map) => map,
            _ => {
                return Err(WorldError::WrongType {
                    x,
                    z,
                    field: "<root>",
                })
            }
        };
        Ok(Self {
            x,
            z,
            data_version: int_field(root, x, z, "DataVersion")?,
            status: string_field(root, x, z, "Status")?,
            sections: sections_of(root, x, z)?,
            root: stored.nbt.clone(),
        })
    }

    /// Checks the full rule for sending this chunk to a client with the
    /// world's `DataVersion`: version match, ready `Status`, and stored
    /// light.
    ///
    /// The caller compiles the reason into a packet or into the
    /// «waits for generation (M6)» list of `docs/PROGRESS.md`; an `Err`
    /// never falls back to an empty or invented chunk.
    pub fn check_sendable(&self, expected_data_version: i32) -> Result<(), ChunkNotReady> {
        if self.data_version != expected_data_version {
            return Err(ChunkNotReady::UnsupportedVersion {
                x: self.x,
                z: self.z,
                found: self.data_version,
                expected: expected_data_version,
            });
        }
        if !READY_STATUSES.contains(&self.status.as_str()) {
            return Err(ChunkNotReady::NotGenerated {
                x: self.x,
                z: self.z,
                status: self.status.clone(),
            });
        }
        let lit = self
            .sections
            .iter()
            .any(|section| section.block_light.is_some() || section.sky_light.is_some());
        if !lit {
            return Err(ChunkNotReady::NoLight {
                x: self.x,
                z: self.z,
            });
        }
        Ok(())
    }
}

/// Reads the `sections` list of a stored chunk.
fn sections_of(
    root: &BTreeMap<String, NbtTag>,
    x: i32,
    z: i32,
) -> Result<Vec<Section>, WorldError> {
    let items = match root.get("sections") {
        Some(NbtTag::List(items)) => items,
        Some(_) => {
            return Err(WorldError::WrongType {
                x,
                z,
                field: "sections",
            })
        }
        None => {
            return Err(WorldError::MissingField {
                x,
                z,
                field: "sections",
            })
        }
    };
    items.iter().map(|item| section_from(item, x, z)).collect()
}

/// Reads one element of the `sections` list.
fn section_from(item: &NbtTag, x: i32, z: i32) -> Result<Section, WorldError> {
    let wrong = |field: &'static str| WorldError::WrongType { x, z, field };
    let map = match item {
        NbtTag::Compound(map) => map,
        _ => return Err(wrong("sections[].<not a compound>")),
    };
    let y = match map.get("Y") {
        Some(NbtTag::Byte(value)) => *value,
        Some(NbtTag::Short(value)) => *value as i8,
        _ => return Err(wrong("sections[].Y")),
    };
    Ok(Section {
        y,
        block_states: map.get("block_states").cloned(),
        biomes: map.get("biomes").cloned(),
        block_light: byte_array(map, "BlockLight"),
        sky_light: byte_array(map, "SkyLight"),
    })
}

/// Reads a byte-array child, `None` when the child is absent.
fn byte_array(map: &BTreeMap<String, NbtTag>, field: &str) -> Option<Vec<i8>> {
    match map.get(field) {
        Some(NbtTag::ByteArray(values)) => Some(values.clone()),
        _ => None,
    }
}

/// Reads an `int` child of a compound.
fn int_field(
    root: &BTreeMap<String, NbtTag>,
    x: i32,
    z: i32,
    field: &'static str,
) -> Result<i32, WorldError> {
    match root.get(field) {
        Some(NbtTag::Int(value)) => Ok(*value),
        Some(_) => Err(WorldError::WrongType { x, z, field }),
        None => Err(WorldError::MissingField { x, z, field }),
    }
}

/// Reads a `string` child of a compound.
fn string_field(
    root: &BTreeMap<String, NbtTag>,
    x: i32,
    z: i32,
    field: &'static str,
) -> Result<String, WorldError> {
    match root.get(field) {
        Some(NbtTag::String(value)) => Ok(value.clone()),
        Some(_) => Err(WorldError::WrongType { x, z, field }),
        None => Err(WorldError::MissingField { x, z, field }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: i32 = 5023;

    /// A minimal stored chunk in the shape the template shows.
    fn stored(status: &str, with_light: bool) -> StoredChunk {
        let section = {
            let mut section = BTreeMap::new();
            section.insert("Y".to_owned(), NbtTag::Byte(-4));
            section.insert("block_states".to_owned(), NbtTag::Compound(BTreeMap::new()));
            if with_light {
                section.insert("BlockLight".to_owned(), NbtTag::ByteArray(vec![15]));
            }
            NbtTag::Compound(section)
        };
        let mut root = BTreeMap::new();
        root.insert("DataVersion".to_owned(), NbtTag::Int(VERSION));
        root.insert("Status".to_owned(), NbtTag::String(status.to_owned()));
        root.insert("sections".to_owned(), NbtTag::List(vec![section]));
        StoredChunk {
            x: 6,
            z: -2,
            compression: 2,
            nbt: NbtTag::Compound(root),
        }
    }

    #[test]
    fn a_ready_chunk_passes_the_rule() {
        let chunk = Chunk::from_stored(&stored("minecraft:full", true)).expect("model");
        assert_eq!(chunk.check_sendable(VERSION), Ok(()));
    }

    #[test]
    fn initialize_light_is_ready_too() {
        let chunk = Chunk::from_stored(&stored("minecraft:initialize_light", true)).expect("model");
        assert_eq!(chunk.check_sendable(VERSION), Ok(()));
    }

    #[test]
    fn an_ungenerated_chunk_names_its_status() {
        let chunk = Chunk::from_stored(&stored("minecraft:structure_starts", true)).expect("model");
        let error = chunk.check_sendable(VERSION).expect_err("not ready");
        assert_eq!(
            error,
            ChunkNotReady::NotGenerated {
                x: 6,
                z: -2,
                status: "minecraft:structure_starts".to_owned(),
            }
        );
        assert!(error.to_string().contains("structure_starts"));
    }

    #[test]
    fn a_ready_status_without_light_is_refused() {
        let chunk = Chunk::from_stored(&stored("minecraft:full", false)).expect("model");
        assert_eq!(
            chunk.check_sendable(VERSION),
            Err(ChunkNotReady::NoLight { x: 6, z: -2 })
        );
    }

    #[test]
    fn a_stored_chunk_of_another_version_is_an_explicit_error() {
        let chunk = Chunk::from_stored(&stored("minecraft:full", true)).expect("model");
        assert_eq!(
            chunk.check_sendable(9999),
            Err(ChunkNotReady::UnsupportedVersion {
                x: 6,
                z: -2,
                found: VERSION,
                expected: 9999,
            })
        );
    }

    #[test]
    fn a_missing_status_is_never_invented() {
        let mut chunk = stored("minecraft:full", true);
        if let NbtTag::Compound(root) = &mut chunk.nbt {
            root.remove("Status");
        }
        let error = Chunk::from_stored(&chunk).expect_err("no Status to read");
        assert!(matches!(
            error,
            WorldError::MissingField {
                field: "Status",
                ..
            }
        ));
    }
}
