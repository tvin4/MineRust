//! Payload records of `level_chunk_with_light`, framed from data.
//!
//! SOURCE: parity trace of vanilla 26.3 — every layout below was read off
//! the nine payloads of the recorded join (walked by the parity tests
//! `blob_walk`, `block_entities` and `light_framing`) rather than from a
//! wiki article. The whole payload is, in order:
//!
//! `[x i32 BE][z i32 BE][VarInt heightmap count][heightmap records]
//! [VarInt section-blob length][section records] [VarInt entity count]
//! [entity records] [light block]`.
//!
//! A section record is `[u16 non-air][u16 fluid][blocks][biomes]`; a
//! container is `[VarInt bits = 0][VarInt value]` when every cell holds one
//! value and `[VarInt bits][VarInt count][count × VarInt id][data]`
//! otherwise, with `data` = `ceil(cells / (64 / bits))` big-endian 64-bit
//! words. The light block is four BitSet masks, a sky and a block group of
//! `VarInt count` arrays of 2048 bytes each.
//!
//! Decoding validates the structural rules the recorded payloads follow
//! (one light array per set bit, disjoint occupied/empty masks, data length
//! implied by the bits); it does not re-derive block ids or biome values —
//! that is what the parity tests do.

use crate::codec::{CodecError, PacketReader, PacketWriter};

/// Cells of one block section (16 × 16 × 16).
pub const BLOCK_CELLS: usize = 4096;

/// Cells of one biome container (the recorded sections carry 64).
pub const BIOME_CELLS: usize = 64;

/// Bytes of one light array: one nibble per block of a 16³ section.
pub const LIGHT_ARRAY_BYTES: usize = 2048;

/// Maximum bits-per-cell a palette container may declare (the recorded
/// sections use 4, 5 and 6 for blocks and 1-3 for biomes).
const MAX_CONTAINER_BITS: i32 = 16;

/// One heightmap of a chunk payload: a wire id and the words of the map.
///
/// The wire carries no name for the id: `registries.json` has no
/// `heightmap_type` registry, so meaning was resolved by byte comparison
/// against the stored chunks (parity test `heightmap_ids`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeightmapRecord {
    /// Wire id of the map (1, 4 and 5 appear in the recordings).
    pub id: i32,
    /// The map as wire words, read big-endian.
    pub words: Vec<i64>,
}

/// One container of a section: either a single value shared by every cell
/// or a palette plus packed cell indices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteContainer {
    /// Every cell holds the given value; no data follows on the wire.
    Single(i32),
    /// `bits` per cell, `palette[value]` decoding the cells, `data` the
    /// packed big-endian words exactly as sent.
    Packed {
        /// Bits per cell (1..=16).
        bits: i32,
        /// Palette of state ids the packed cells index into.
        palette: Vec<i32>,
        /// Packed cells, big-endian words, length fixed by `bits`.
        data: Vec<u8>,
    },
}

/// One section record inside the section blob, in pack order (bottom to top).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkSection {
    /// Header count of non-air cells in this section.
    pub non_air: u16,
    /// Header count of fluid cells in this section.
    pub fluid: u16,
    /// Block states of the 4096 cells.
    pub blocks: PaletteContainer,
    /// Biomes of the 64 biome cells.
    pub biomes: PaletteContainer,
}

/// One block entity record: local coordinates, type id and its raw NBT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockEntity {
    /// Local block x inside the chunk (0..=15).
    pub local_x: u8,
    /// Local block z inside the chunk (0..=15).
    pub local_z: u8,
    /// Block y of the entity.
    pub y: i16,
    /// `minecraft:block_entity_type` id from `registries.json`.
    pub type_id: i32,
    /// Its NBT exactly as sent: one `00` byte for an empty compound,
    /// otherwise a network NBT compound.
    pub nbt: Vec<u8>,
}

/// The light block: four BitSet masks as sent, then both array groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LightData {
    /// Sections with a packed sky light array.
    pub sky_mask: Vec<u8>,
    /// Sections with a packed block light array.
    pub block_mask: Vec<u8>,
    /// Sections whose sky light is all zeros (no array sent).
    pub empty_sky_mask: Vec<u8>,
    /// Sections whose block light is all zeros (no array sent).
    pub empty_block_mask: Vec<u8>,
    /// Sky light arrays, one per set bit of `sky_mask`, 2048 bytes each.
    pub sky_arrays: Vec<Vec<u8>>,
    /// Block light arrays, one per set bit of `block_mask`.
    pub block_arrays: Vec<Vec<u8>>,
}

/// Set bits of a BitSet payload as Java `BitSet.toByteArray` orders them:
/// byte `i` carries the bits `8i..8i+7`, least significant bit first.
pub fn set_bits(mask: &[u8]) -> Vec<usize> {
    let mut bits = Vec::new();
    for (index, byte) in mask.iter().enumerate() {
        for branch in 0..8_usize {
            if byte >> branch & 1 != 0 {
                bits.push(index * 8 + branch);
            }
        }
    }
    bits
}

/// Reads the heightmap group: VarInt count, then records of
/// `[VarInt id][VarInt word count][words]`.
pub(crate) fn read_heightmaps(
    reader: &mut PacketReader<'_>,
) -> Result<Vec<HeightmapRecord>, CodecError> {
    let count = reader.read_varint()?;
    // The recordings carry exactly three maps per chunk; eight is the
    // largest count the layout can be meant for.
    if !(1..=8).contains(&count) {
        return Err(CodecError::InvalidChunk("heightmap count"));
    }
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let id = reader.read_varint()?;
        if id < 0 {
            return Err(CodecError::InvalidChunk("negative heightmap id"));
        }
        let words = reader.read_varint()?;
        if words < 0 {
            return Err(CodecError::InvalidChunk("negative heightmap length"));
        }
        let mut values = Vec::with_capacity(words as usize);
        for _ in 0..words {
            values.push(reader.read_i64()?);
        }
        out.push(HeightmapRecord { id, words: values });
    }
    Ok(out)
}

/// Writes the heightmap group back exactly as it is framed.
pub(crate) fn write_heightmaps(
    writer: &mut PacketWriter,
    maps: &[HeightmapRecord],
) -> Result<(), CodecError> {
    if !(1..=8).contains(&(maps.len() as i32)) {
        return Err(CodecError::InvalidChunk("heightmap count"));
    }
    writer.write_varint(maps.len() as i32);
    for map in maps {
        if map.id < 0 || map.words.len() > i32::MAX as usize {
            return Err(CodecError::InvalidChunk("heightmap record"));
        }
        writer.write_varint(map.id);
        writer.write_varint(map.words.len() as i32);
        for word in &map.words {
            writer.write_i64(*word);
        }
    }
    Ok(())
}

/// Reads the section blob: one length prefix, then section records until
/// it is consumed exactly.
pub(crate) fn read_sections(
    reader: &mut PacketReader<'_>,
) -> Result<Vec<ChunkSection>, CodecError> {
    let len = reader.read_varint()?;
    let len = usize::try_from(len).map_err(|_| CodecError::NegativeLength(len))?;
    let blob = reader.read_bytes(len)?;
    let mut cursor = PacketReader::new(blob);
    let mut sections = Vec::new();
    while !cursor.is_empty() {
        sections.push(read_section(&mut cursor)?);
    }
    Ok(sections)
}

/// Writes the section blob: the prefix carries the byte length the records
/// turn out to have, so the blob is built first and appended after.
pub(crate) fn write_sections(
    writer: &mut PacketWriter,
    sections: &[ChunkSection],
) -> Result<(), CodecError> {
    let mut blob = PacketWriter::new();
    for section in sections {
        write_section(&mut blob, section)?;
    }
    let bytes = blob.into_bytes();
    let len = i32::try_from(bytes.len()).map_err(|_| CodecError::InvalidChunk("section blob"))?;
    writer.write_varint(len);
    writer.write_bytes(&bytes);
    Ok(())
}

/// Reads one section record: two u16 headers, then both containers.
fn read_section(reader: &mut PacketReader<'_>) -> Result<ChunkSection, CodecError> {
    let non_air = reader.read_u16()?;
    let fluid = reader.read_u16()?;
    let blocks = read_container(reader, BLOCK_CELLS)?;
    let biomes = read_container(reader, BIOME_CELLS)?;
    Ok(ChunkSection {
        non_air,
        fluid,
        blocks,
        biomes,
    })
}

/// Writes one section record.
fn write_section(writer: &mut PacketWriter, section: &ChunkSection) -> Result<(), CodecError> {
    writer.write_u16(section.non_air);
    writer.write_u16(section.fluid);
    write_container(writer, &section.blocks, BLOCK_CELLS)?;
    write_container(writer, &section.biomes, BIOME_CELLS)
}

/// Reads one container of `cells` cells: a single value when the bits are
/// zero, otherwise the palette and the packed data of a size implied by
/// the bits (there is no length prefix for the data).
fn read_container(
    reader: &mut PacketReader<'_>,
    cells: usize,
) -> Result<PaletteContainer, CodecError> {
    let bits = reader.read_varint()?;
    if bits == 0 {
        return Ok(PaletteContainer::Single(reader.read_varint()?));
    }
    if !(1..=MAX_CONTAINER_BITS).contains(&bits) {
        return Err(CodecError::InvalidChunk("palette bits"));
    }
    let count = reader.read_varint()?;
    let count_usize =
        usize::try_from(count).map_err(|_| CodecError::InvalidChunk("palette size"))?;
    if count_usize > cells || count_usize == 0 {
        return Err(CodecError::InvalidChunk("palette size"));
    }
    let mut palette = Vec::with_capacity(count_usize);
    for _ in 0..count_usize {
        palette.push(reader.read_varint()?);
    }
    let data = reader.read_bytes(data_len(bits, cells)?)?.to_vec();
    Ok(PaletteContainer::Packed {
        bits,
        palette,
        data,
    })
}

/// Writes one container: form, bits, palette and the packed data, whose
/// length must already match the bits it declares.
fn write_container(
    writer: &mut PacketWriter,
    container: &PaletteContainer,
    cells: usize,
) -> Result<(), CodecError> {
    match container {
        PaletteContainer::Single(value) => {
            writer.write_varint(0);
            writer.write_varint(*value);
            Ok(())
        }
        PaletteContainer::Packed {
            bits,
            palette,
            data,
        } => {
            if !(1..=MAX_CONTAINER_BITS).contains(bits) {
                return Err(CodecError::InvalidChunk("palette bits"));
            }
            if palette.is_empty() || palette.len() > cells {
                return Err(CodecError::InvalidChunk("palette size"));
            }
            if data.len() != data_len(*bits, cells)? {
                return Err(CodecError::InvalidChunk("packed data length"));
            }
            writer.write_varint(*bits);
            writer.write_varint(palette.len() as i32);
            for id in palette {
                writer.write_varint(*id);
            }
            writer.write_bytes(data);
            Ok(())
        }
    }
}

/// Byte length of the packed data of `cells` cells at `bits` per cell:
/// one word holds `64 / bits` cells, and the last word is padded.
fn data_len(bits: i32, cells: usize) -> Result<usize, CodecError> {
    let per_word =
        64 / usize::try_from(bits).map_err(|_| CodecError::InvalidChunk("palette bits"))?;
    Ok(cells.div_ceil(per_word) * 8)
}

/// Reads the block entity group: VarInt count, then records of
/// `[u8 packed xz][i16 y][VarInt type id][NBT record]`.
pub(crate) fn read_block_entities(
    reader: &mut PacketReader<'_>,
) -> Result<Vec<BlockEntity>, CodecError> {
    let count = reader.read_varint()?;
    if count < 0 {
        return Err(CodecError::InvalidChunk("negative block entity count"));
    }
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let packed = reader.read_u8()?;
        let y = reader.read_u16()? as i16;
        let type_id = reader.read_varint()?;
        let nbt = reader.read_nbt_bytes()?.to_vec();
        out.push(BlockEntity {
            local_x: packed >> 4,
            local_z: packed & 0x0f,
            y,
            type_id,
            nbt,
        });
    }
    Ok(out)
}

/// Writes the block entity group back as it is framed.
pub(crate) fn write_block_entities(
    writer: &mut PacketWriter,
    entities: &[BlockEntity],
) -> Result<(), CodecError> {
    let count =
        i32::try_from(entities.len()).map_err(|_| CodecError::InvalidChunk("block entities"))?;
    writer.write_varint(count);
    for entity in entities {
        if entity.local_x >= 16 || entity.local_z >= 16 {
            return Err(CodecError::InvalidChunk("block entity outside its chunk"));
        }
        let packed = (entity.local_x << 4) | entity.local_z;
        writer.write_u8(packed);
        writer.write_u16(entity.y as u16);
        writer.write_varint(entity.type_id);
        writer.write_bytes(&entity.nbt);
    }
    Ok(())
}

/// Reads the light block: four masks, then the sky and block arrays.
pub(crate) fn read_light(reader: &mut PacketReader<'_>) -> Result<LightData, CodecError> {
    let sky_mask = read_bitset(reader)?;
    let block_mask = read_bitset(reader)?;
    let empty_sky_mask = read_bitset(reader)?;
    let empty_block_mask = read_bitset(reader)?;
    let sky_arrays = read_arrays(reader)?;
    let block_arrays = read_arrays(reader)?;
    let light = LightData {
        sky_mask,
        block_mask,
        empty_sky_mask,
        empty_block_mask,
        sky_arrays,
        block_arrays,
    };
    check_light(&light)?;
    Ok(light)
}

/// Writes the light block after checking the same rules decoding relies on.
pub(crate) fn write_light(writer: &mut PacketWriter, light: &LightData) -> Result<(), CodecError> {
    check_light(light)?;
    for mask in [
        &light.sky_mask,
        &light.block_mask,
        &light.empty_sky_mask,
        &light.empty_block_mask,
    ] {
        write_bitset(writer, mask);
    }
    write_arrays(writer, &light.sky_arrays)?;
    write_arrays(writer, &light.block_arrays)
}

/// Structural rules of the light block, derived from all nine recorded
/// payloads: each chain has exactly one array per occupied section, and a
/// section is either occupied or marked empty in the same chain, never
/// both. Breaks are reported as [`CodecError::InvalidChunk`].
fn check_light(light: &LightData) -> Result<(), CodecError> {
    let sky = set_bits(&light.sky_mask);
    let block = set_bits(&light.block_mask);
    if sky.len() != light.sky_arrays.len() {
        return Err(CodecError::InvalidChunk("sky mask vs array count"));
    }
    if block.len() != light.block_arrays.len() {
        return Err(CodecError::InvalidChunk("block mask vs array count"));
    }
    for (occupied, empty) in [
        (&sky, set_bits(&light.empty_sky_mask)),
        (&block, set_bits(&light.empty_block_mask)),
    ] {
        if occupied.iter().any(|bit| empty.contains(bit)) {
            return Err(CodecError::InvalidChunk("section occupied and empty"));
        }
    }
    for array in &light.sky_arrays {
        if array.len() != LIGHT_ARRAY_BYTES {
            return Err(CodecError::InvalidChunk("sky array length"));
        }
    }
    for array in &light.block_arrays {
        if array.len() != LIGHT_ARRAY_BYTES {
            return Err(CodecError::InvalidChunk("block array length"));
        }
    }
    Ok(())
}

/// Reads one BitSet as its raw bytes: `VarInt length` then the payload.
/// The raw form is kept because parity compares the bytes it received.
fn read_bitset(reader: &mut PacketReader<'_>) -> Result<Vec<u8>, CodecError> {
    let len = reader.read_varint()?;
    let len = usize::try_from(len).map_err(|_| CodecError::NegativeLength(len))?;
    Ok(reader.read_bytes(len)?.to_vec())
}

/// Writes one BitSet as `VarInt length` plus payload.
fn write_bitset(writer: &mut PacketWriter, mask: &[u8]) {
    writer.write_varint(mask.len() as i32);
    writer.write_bytes(mask);
}

/// Reads one light array group: `VarInt count` then arrays of a fixed
/// 2048-byte length, which is the framing every recording carries.
fn read_arrays(reader: &mut PacketReader<'_>) -> Result<Vec<Vec<u8>>, CodecError> {
    let count = reader.read_varint()?;
    if count < 0 {
        return Err(CodecError::InvalidChunk("negative light array count"));
    }
    let mut arrays = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let len = reader.read_varint()?;
        if len != LIGHT_ARRAY_BYTES as i32 {
            return Err(CodecError::InvalidChunk("light array length"));
        }
        arrays.push(reader.read_bytes(LIGHT_ARRAY_BYTES)?.to_vec());
    }
    Ok(arrays)
}

/// Writes one light array group.
fn write_arrays(writer: &mut PacketWriter, arrays: &[Vec<u8>]) -> Result<(), CodecError> {
    let count =
        i32::try_from(arrays.len()).map_err(|_| CodecError::InvalidChunk("light arrays"))?;
    writer.write_varint(count);
    for array in arrays {
        if array.len() != LIGHT_ARRAY_BYTES {
            return Err(CodecError::InvalidChunk("light array length"));
        }
        writer.write_varint(LIGHT_ARRAY_BYTES as i32);
        writer.write_bytes(array);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_length_follows_the_bits() {
        // The three sizes the recorded sections use.
        assert_eq!(data_len(4, BLOCK_CELLS).expect("4 bits"), 2048);
        assert_eq!(data_len(5, BLOCK_CELLS).expect("5 bits"), 2736);
        assert_eq!(data_len(6, BLOCK_CELLS).expect("6 bits"), 3280);
        assert_eq!(data_len(1, BIOME_CELLS).expect("biome 1 bit"), 8);
    }

    #[test]
    fn set_bits_follow_java_bitset_order() {
        // Byte 1 set: bit 8, not bit 1.
        assert_eq!(set_bits(&[0x00, 0x01]), vec![8]);
        assert_eq!(set_bits(&[0x03]), vec![0, 1]);
        assert!(set_bits(&[]).is_empty());
    }

    #[test]
    fn a_container_roundtrips_both_forms() {
        for container in [
            PaletteContainer::Single(41),
            PaletteContainer::Packed {
                bits: 6,
                palette: vec![1, 2, 3],
                data: vec![0xFF; 3280],
            },
        ] {
            let mut writer = PacketWriter::new();
            write_container(&mut writer, &container, BLOCK_CELLS).expect("encodable");
            let mut reader = PacketReader::new(writer.as_bytes());
            let decoded = read_container(&mut reader, BLOCK_CELLS).expect("decodable");
            assert_eq!(decoded, container);
            reader.ensure_empty().expect("no trailing bytes");
        }
    }

    #[test]
    fn packed_data_that_contradicts_the_bits_is_rejected() {
        let broken = PaletteContainer::Packed {
            bits: 4,
            palette: vec![0],
            data: vec![0; 7],
        };
        let mut writer = PacketWriter::new();
        let err = write_container(&mut writer, &broken, BLOCK_CELLS)
            .expect_err("data length comes from the bits");
        assert!(matches!(err, CodecError::InvalidChunk(_)), "{err}");
    }

    #[test]
    fn palette_bits_outside_the_range_are_rejected() {
        // The bits check must fire before anything tries to read a palette
        // or a data array behind them.
        for bits in [17_i32, -1] {
            let mut writer = PacketWriter::new();
            writer.write_varint(bits);
            writer.write_varint(1);
            writer.write_varint(0);
            let mut reader = PacketReader::new(writer.as_bytes());
            let err = read_container(&mut reader, BLOCK_CELLS).expect_err("unsupported bits");
            assert!(matches!(err, CodecError::InvalidChunk(_)), "{bits}: {err}");
        }
    }

    #[test]
    fn light_masks_must_agree_with_the_array_counts() {
        let light = LightData {
            sky_mask: vec![0x03], // two occupied sections
            block_mask: Vec::new(),
            empty_sky_mask: Vec::new(),
            empty_block_mask: Vec::new(),
            sky_arrays: vec![vec![0; LIGHT_ARRAY_BYTES]], // but one array
            block_arrays: Vec::new(),
        };
        let err = check_light(&light).expect_err("one array per mask bit");
        assert!(matches!(err, CodecError::InvalidChunk(_)), "{err}");
    }

    #[test]
    fn a_section_cannot_be_occupied_and_empty_at_once() {
        let light = LightData {
            sky_mask: vec![0x01],
            block_mask: Vec::new(),
            empty_sky_mask: vec![0x01],
            empty_block_mask: Vec::new(),
            sky_arrays: vec![vec![0; LIGHT_ARRAY_BYTES]],
            block_arrays: Vec::new(),
        };
        let err = check_light(&light).expect_err("the same section in both masks");
        assert!(matches!(err, CodecError::InvalidChunk(_)), "{err}");
    }

    #[test]
    fn block_entities_roundtrip_with_both_nbt_forms() {
        let entities = vec![
            BlockEntity {
                local_x: 6,
                local_z: 9,
                y: -30,
                type_id: 34,
                nbt: vec![0x00],
            },
            BlockEntity {
                local_x: 0,
                local_z: 15,
                y: 300,
                type_id: 9,
                nbt: crate::NbtTag::Compound(std::collections::BTreeMap::from([(
                    "id".to_owned(),
                    crate::NbtTag::String("minecraft:mob_spawner".to_owned()),
                )]))
                .to_network_bytes()
                .expect("encodable"),
            },
        ];
        let mut writer = PacketWriter::new();
        write_block_entities(&mut writer, &entities).expect("encodable");
        let mut reader = PacketReader::new(writer.as_bytes());
        let decoded = read_block_entities(&mut reader).expect("decodable");
        assert_eq!(decoded, entities);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn an_entity_outside_its_chunk_is_rejected() {
        let entity = BlockEntity {
            local_x: 16,
            local_z: 0,
            y: 0,
            type_id: 1,
            nbt: vec![0x00],
        };
        let mut writer = PacketWriter::new();
        let err = write_block_entities(&mut writer, &[entity]).expect_err("x must be a nibble");
        assert!(matches!(err, CodecError::InvalidChunk(_)), "{err}");
    }

    #[test]
    fn a_zero_length_light_array_is_not_accepted() {
        let mut writer = PacketWriter::new();
        writer.write_varint(1);
        writer.write_varint(0); // length prefix contradicts the 2048 rule
        let mut reader = PacketReader::new(writer.as_bytes());
        let err = read_arrays(&mut reader).expect_err("fixed array size");
        assert!(matches!(err, CodecError::InvalidChunk(_)), "{err}");
    }
}
