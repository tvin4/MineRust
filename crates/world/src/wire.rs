//! Converting a stored [`Chunk`] into the payload of
//! `minecraft:level_chunk_with_light`.
//!
//! Every value of the payload is derived from data, never from memory:
//!
//! * state ids, block entity type ids and biome ids come from the reports of
//!   the user's own game copy through [`WireRefs`] (block state ids matched
//!   the recorded sections in parity test `section_map`, block entity type
//!   ids in `block_entities`, biome ids were read off the recorded Registry
//!   Data packet);
//! * heightmap records carry the stored `Heightmaps` longs under the ids the
//!   recording pinned by byte equality (parity test `heightmap_ids`);
//! * light follows the recorded choices of `docs/DECISIONS.md` (§141): a
//!   stored `SkyLight`/`BlockLight` array goes on the wire as that section's
//!   array, a section without stored light is sent as sky 15 / block 0.
//!
//! Light arrays are framed by `wire_light`, sections by `wire_section` and
//! block entity records by `wire_block_entity`. Differences the vanilla
//! recording already shows against the stored template (block entity order,
//! light arrays of not-fully-generated chunks) are reported by the chunk
//! parity test of step 3.3f, never guessed away here.

use std::collections::BTreeMap;

use protocol::{HeightmapRecord, LevelChunkWithLight, NbtTag};

use crate::chunk::Chunk;

/// Lookups a chunk-to-wire conversion needs; the caller binds them to the
/// data reports of the user's own game copy.
///
/// The stored model keeps names (`minecraft:sculk_sensor`, `minecraft:plains`);
/// the wire palette keeps numeric ids. Which report each id comes from is
/// observed data, not convention:
///
/// * a block state id comes from `reports/blocks.json` — the state whose
///   property values match the stored entry, or the block default;
/// * a block entity type id comes from `reports/registries.json`
///   (`minecraft:block_entity_type`);
/// * a biome id is its position inside the biome registry of the Registry
///   Data packet — confirmed on the recorded join: the stored palettes
///   `dark_forest`, `deep_dark` and `lush_caves` appear on the wire as 9,
///   11 and 31, the alphabetical positions those names hold in the packet.
pub trait WireRefs {
    /// State id of a palette entry: the block default when no properties are
    /// stored, otherwise the state of that block matching every property.
    fn block_state_id(
        &self,
        name: &str,
        properties: Option<&BTreeMap<String, String>>,
    ) -> Option<i32>;

    /// `minecraft:block_entity_type` id of `name` (registries report).
    fn block_entity_type_id(&self, name: &str) -> Option<i32>;

    /// Id of a biome in the Registry Data packet (the synchronized
    /// `minecraft:worldgen/biome` registry of the game jar).
    fn biome_id(&self, name: &str) -> Option<i32>;
}

/// Why a conversion names data instead of converting; every variant points
/// at a stored field or a report entry that was actually read.
#[derive(Debug, thiserror::Error)]
pub enum WireError {
    /// A stored value is not the shape the conversion reads.
    #[error("chunk ({x}, {z}): {field} is not the expected NBT")]
    WrongType {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
        /// Field path that failed, e.g. `section Y=3 block_states.palette`.
        field: String,
    },
    /// The chunk carries fewer sections than the world has slots.
    #[error("chunk ({x}, {z}): no section Y={y}")]
    MissingSection {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
        /// Section Y that is absent.
        y: i8,
    },
    /// A palette entry names a block the blocks report does not know.
    #[error("chunk ({x}, {z}) section Y={y}: block {name} is not in blocks.json")]
    UnknownBlock {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
        /// Section Y of the palette.
        y: i8,
        /// Block name the report lacks.
        name: String,
    },
    /// A palette entry names a biome the game data does not synchronize.
    #[error("chunk ({x}, {z}) section Y={y}: biome {name} is not in the game data")]
    UnknownBiome {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
        /// Section Y of the palette.
        y: i8,
        /// Biome name the game data lacks.
        name: String,
    },
    /// A block entity names a type the registries report does not know.
    #[error("chunk ({x}, {z}): block entity type {name} is not in registries.json")]
    UnknownBlockEntity {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
        /// Type name the report lacks.
        name: String,
    },
    /// The stored heightmap list has no such map (or it is not a long array).
    #[error("chunk ({x}, {z}): heightmap {key} is missing or not a long array")]
    HeightmapMissing {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
        /// Stored `Heightmaps` key.
        key: &'static str,
    },
    /// Packed `data` length contradicts the bits its palette implies.
    #[error(
        "chunk ({x}, {z}) section Y={y}: {field} packs {longs} longs for {bits} bits, \
         expected {expected}"
    )]
    PackedLength {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
        /// Section Y.
        y: i8,
        /// Field name (`block_states`/`biomes`).
        field: &'static str,
        /// Longs stored.
        longs: usize,
        /// Bits per cell the palette implies.
        bits: usize,
        /// Longs the cell count needs at those bits.
        expected: usize,
    },
    /// A block entity coordinate does not fit the record (`[u8 xz][i16 y]`).
    #[error("chunk ({chunk_x}, {chunk_z}): block entity at ({x}, {y}, {z}) does not fit a record")]
    EntityPosition {
        /// Chunk x.
        chunk_x: i32,
        /// Chunk z.
        chunk_z: i32,
        /// Stored world x of the entity.
        x: i32,
        /// Stored world y of the entity.
        y: i32,
        /// Stored world z of the entity.
        z: i32,
    },
    /// A stored light array is not the 2048 bytes the wire carries.
    #[error("chunk ({x}, {z}) section Y={y}: {field} holds {bytes} bytes, expected 2048")]
    LightLength {
        /// Chunk x.
        x: i32,
        /// Chunk z.
        z: i32,
        /// Section Y the array belongs to.
        y: i8,
        /// Field name (`SkyLight`/`BlockLight`).
        field: &'static str,
        /// Bytes stored.
        bytes: usize,
    },
    /// The record's network NBT could not be encoded.
    #[error("block entity NBT could not be encoded: {0}")]
    Nbt(#[from] protocol::NbtError),
}

/// The three heightmap records every recorded payload carries, in the order
/// run 1 and the main run of the recording send them.
///
/// SOURCE: parity trace — ids were pinned by byte equality against the
/// stored maps (parity test `heightmap_ids`; the reports carry no
/// `heightmap_type` registry to read the ids from). The same recording's
/// run 3 lists the same three records in a different order (Java map
/// iteration), so the order here follows the majority run and is reported
/// by the parity test if it ever differs.
const HEIGHTMAPS: [(&str, i32); 3] = [
    ("MOTION_BLOCKING", 4),
    ("MOTION_BLOCKING_NO_LEAVES", 5),
    ("WORLD_SURFACE", 1),
];

impl Chunk {
    /// Converts this stored chunk into a `level_chunk_with_light` payload.
    ///
    /// The caller must have checked [`Self::check_sendable`] first: this
    /// method encodes what the section/light/status rule accepted and fails
    /// with a [`WireError`] naming stored data instead of inventing a
    /// default.
    pub fn level_chunk(&self, refs: &dyn WireRefs) -> Result<LevelChunkWithLight, WireError> {
        let heightmaps = self.heightmap_records()?;
        let sections = crate::wire_section::prepare(self, refs)?;
        let block_entities = crate::wire_block_entity::records(self, &sections, refs)?;
        let light = crate::wire_light::light(self)?;
        Ok(LevelChunkWithLight {
            chunk_x: self.x,
            chunk_z: self.z,
            heightmaps,
            sections: sections
                .iter()
                .map(|section| section.record.clone())
                .collect(),
            block_entities,
            light,
        })
    }

    /// The heightmap group: stored longs under the observed wire ids.
    fn heightmap_records(&self) -> Result<Vec<HeightmapRecord>, WireError> {
        let maps = match &self.root {
            NbtTag::Compound(root) => match root.get("Heightmaps") {
                Some(NbtTag::Compound(maps)) => maps,
                _ => {
                    return Err(self.wrong_type("Heightmaps"));
                }
            },
            _ => {
                return Err(self.wrong_type("<root>"));
            }
        };
        HEIGHTMAPS
            .iter()
            .map(|(key, id)| match maps.get(*key) {
                Some(NbtTag::LongArray(words)) => Ok(HeightmapRecord {
                    id: *id,
                    words: words.clone(),
                }),
                _ => Err(WireError::HeightmapMissing {
                    x: self.x,
                    z: self.z,
                    key,
                }),
            })
            .collect()
    }

    /// The standard error text for a root field of the wrong shape.
    pub(crate) fn wrong_type(&self, field: &str) -> WireError {
        WireError::WrongType {
            x: self.x,
            z: self.z,
            field: field.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::wire_fixture::{self, TestRefs};
    use protocol::{PacketReader, PaletteContainer};

    #[test]
    fn heightmaps_carry_the_stored_longs_under_the_recorded_ids() {
        let chunk = wire_fixture::chunk();
        let records = chunk.heightmap_records().expect("fixture has the maps");
        assert_eq!(
            records.iter().map(|record| record.id).collect::<Vec<_>>(),
            vec![4, 5, 1],
            "the wire order of the recorded runs"
        );
        assert_eq!(records[0].words, vec![11, 22], "stored words untouched");
        assert_eq!(records[1].words, vec![33], "MOTION_BLOCKING_NO_LEAVES");
        assert_eq!(records[2].words, vec![44], "WORLD_SURFACE");
    }

    #[test]
    fn a_missing_heightmap_is_named_instead_of_defaulted() {
        let mut chunk = wire_fixture::chunk();
        if let protocol::NbtTag::Compound(root) = &mut chunk.root {
            if let Some(protocol::NbtTag::Compound(maps)) = root.get_mut("Heightmaps") {
                maps.remove("WORLD_SURFACE");
            }
        }
        let error = chunk.heightmap_records().expect_err("map removed");
        assert!(
            matches!(
                error,
                crate::WireError::HeightmapMissing {
                    key: "WORLD_SURFACE",
                    ..
                }
            ),
            "{error}"
        );
    }

    #[test]
    fn a_converted_chunk_encodes_and_decodes_again() {
        let chunk = wire_fixture::chunk();
        let payload = chunk
            .level_chunk(&TestRefs)
            .expect("fixture converts")
            .encode_payload()
            .expect("encodable");
        let decoded =
            protocol::LevelChunkWithLight::decode_payload(&mut PacketReader::new(&payload))
                .expect("the codec accepts what we built");
        assert_eq!(decoded.chunk_x, 6);
        assert_eq!(decoded.chunk_z, -2);
        assert_eq!(decoded.sections.len(), wire_fixture::SECTION_SLOTS);
        assert_eq!(decoded.heightmaps.len(), 3);
        assert_eq!(
            decoded.sections[0].blocks,
            PaletteContainer::Single(wire_fixture::AIR),
            "bottom section is plain air"
        );
        assert_eq!(decoded.block_entities.len(), 0);
        // The light rule of DECISIONS §141: one sky array per section, all
        // of them the synthesized 15 for a chunk without stored light.
        assert_eq!(decoded.light.sky_arrays.len(), wire_fixture::SECTION_SLOTS);
        assert!(decoded
            .light
            .sky_arrays
            .iter()
            .all(|array| array.iter().all(|&byte| byte == 0xFF)));
        assert_eq!(decoded.light.sky_mask, decoded.light.empty_block_mask);
    }
}
