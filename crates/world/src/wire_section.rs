//! Section records of the chunk payload: palettes, packed cells and the
//! two count headers of each section.
//!
//! SOURCE: the stored `block_states`/`biomes` compounds as the template
//! writes them, decoded under the formulas the parity step confirmed on the
//! full chunk (6, −2): bits for blocks = `max(4, ceil(log2(palette)))`,
//! bits for biomes = `ceil(log2(palette))`, cells packed least significant
//! bit first inside big-endian longs — every one of its 24 sections then
//! reproduced the recorded bytes exactly (`parity/section_map`).
//!
//! A palette entry the reports cannot resolve is an error with the name in
//! it: a section is never re-encoded from a guessed id.

use std::collections::BTreeMap;

use protocol::{ChunkSection, NbtTag, PaletteContainer, BIOME_CELLS, BLOCK_CELLS};

use crate::chunk::{Chunk, Section};
use crate::wire::{WireError, WireRefs};

/// The stored template fills every section slot of its dimension: Y −4 … 19.
///
/// SOURCE: the owner's template, 24 sections per chunk in all nine sent
/// chunks (`world_inspect`); a chunk stored without one of these slots is a
/// named error — the record set is data, not a default.
pub(crate) const SLOT: (i8, i8) = (-4, 19);

/// How many slots the dimension has: 24 sections, also the size of the
/// light masks (`wire_light` frames them from the same numbers).
pub(crate) const SLOT_COUNT: usize = (SLOT.1 - SLOT.0) as usize + 1;

/// One section prepared for the wire: the record itself plus the decoded
/// view block entity records look the block under their coordinate up with.
#[derive(Debug)]
pub(crate) struct Prepared {
    /// Section Y the record stands for.
    pub(crate) y: i8,
    /// Block names of the palette; a cell index points into this slice.
    pub(crate) names: Vec<String>,
    /// Palette index of every one of the 4096 cells.
    pub(crate) cells: Vec<usize>,
    /// The wire record of this section.
    pub(crate) record: ChunkSection,
}

/// One stored palette entry: its name and — for a state that differs from
/// the default — the properties that select the state.
struct StoredEntry {
    /// Block or biome name without properties.
    name: String,
    /// Property values of a non-default block state.
    properties: Option<BTreeMap<String, String>>,
}

/// The section records of a chunk, bottom to top, one per stored slot.
pub(crate) fn prepare(chunk: &Chunk, refs: &dyn WireRefs) -> Result<Vec<Prepared>, WireError> {
    let mut stored = BTreeMap::new();
    for section in &chunk.sections {
        stored.insert(section.y, section);
        if !(SLOT.0..=SLOT.1).contains(&section.y) {
            return Err(chunk.wrong_type(&format!(
                "section Y={} is outside the slots Y={}..Y={}",
                section.y, SLOT.0, SLOT.1
            )));
        }
    }
    if stored.len() != chunk.sections.len() {
        return Err(chunk.wrong_type("sections with duplicate Y"));
    }
    let mut out = Vec::with_capacity(SLOT_COUNT);
    for y in SLOT.0..=SLOT.1 {
        let section = stored.get(&y).ok_or(WireError::MissingSection {
            x: chunk.x,
            z: chunk.z,
            y,
        })?;
        out.push(section_record(chunk, section, y, refs)?);
    }
    Ok(out)
}

/// Prepares one stored section: both containers, then the two headers.
fn section_record(
    chunk: &Chunk,
    section: &Section,
    y: i8,
    refs: &dyn WireRefs,
) -> Result<Prepared, WireError> {
    let states = named_compound(chunk, y, section.block_states.as_ref(), "block_states")?;
    let (blocks, names, cells) = container(
        chunk,
        y,
        "block_states",
        states,
        BLOCK_CELLS,
        4,
        |name, props| {
            refs.block_state_id(name, props)
                .ok_or_else(|| WireError::UnknownBlock {
                    x: chunk.x,
                    z: chunk.z,
                    y,
                    name: name.to_owned(),
                })
        },
    )?;
    let biomes = named_compound(chunk, y, section.biomes.as_ref(), "biomes")?;
    let (biomes, _, _) = container(chunk, y, "biomes", biomes, BIOME_CELLS, 1, |name, _| {
        refs.biome_id(name).ok_or_else(|| WireError::UnknownBiome {
            x: chunk.x,
            z: chunk.z,
            y,
            name: name.to_owned(),
        })
    })?;
    let (non_air, fluid) = counts(&names, &cells);
    Ok(Prepared {
        y,
        names,
        cells,
        record: ChunkSection {
            non_air,
            fluid,
            blocks,
            biomes,
        },
    })
}

/// The named compound child of a section, or the standard shape error.
fn named_compound<'a>(
    chunk: &Chunk,
    y: i8,
    value: Option<&'a NbtTag>,
    field: &str,
) -> Result<&'a BTreeMap<String, NbtTag>, WireError> {
    match value {
        Some(NbtTag::Compound(map)) => Ok(map),
        _ => Err(chunk.wrong_type(&format!("section Y={y} {field}"))),
    }
}

/// One palette container of a section: the wire record, the palette names
/// and — for blocks — the palette index of every cell.
///
/// `min_bits` is the recorded floor (4 for blocks, 1 for biomes); `cells`
/// fixes the cell count (`4096` blocks, `64` biomes); `to_id` resolves one
/// entry into the wire id and names its own error when the report has no
/// such entry.
fn container(
    chunk: &Chunk,
    y: i8,
    field: &'static str,
    compound: &BTreeMap<String, NbtTag>,
    cells: usize,
    min_bits: usize,
    to_id: impl Fn(&str, Option<&BTreeMap<String, String>>) -> Result<i32, WireError>,
) -> Result<(PaletteContainer, Vec<String>, Vec<usize>), WireError> {
    let entries = palette_entries(chunk, y, field, compound)?;
    if entries.len() > cells {
        return Err(chunk.wrong_type(&format!(
            "section Y={y} {field}.palette holds {} entries for {cells} cells",
            entries.len()
        )));
    }
    let names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
    let ids = entries
        .iter()
        .map(|entry| to_id(&entry.name, entry.properties.as_ref()))
        .collect::<Result<Vec<i32>, WireError>>()?;
    let Some(NbtTag::LongArray(words)) = compound.get("data") else {
        // Without `data` the container is single-valued: the recorded
        // payloads prove this only for a one-entry palette.
        if names.len() != 1 {
            return Err(chunk.wrong_type(&format!(
                "section Y={y} {field} has no data for {} entries",
                names.len()
            )));
        }
        return Ok((PaletteContainer::Single(ids[0]), names, vec![0; cells]));
    };
    let bits = cell_bits(names.len(), min_bits);
    let expected = longs_for(cells, bits);
    if words.len() != expected {
        return Err(WireError::PackedLength {
            x: chunk.x,
            z: chunk.z,
            y,
            field,
            longs: words.len(),
            bits,
            expected,
        });
    }
    let indices = unpack(words, bits, cells)?;
    if let Some(index) = indices.iter().find(|&&index| index >= names.len()) {
        return Err(chunk.wrong_type(&format!(
            "section Y={y} {field} data points at palette index {index} of {} entries",
            names.len()
        )));
    }
    Ok((
        PaletteContainer::Packed {
            bits: bits as i32,
            palette: ids,
            data: pack(&indices, bits),
        },
        names,
        indices,
    ))
}

/// Reads the `palette` list of one stored container. The template mixes
/// three shapes: a plain string, a compound with the name under `id`,
/// `Name` or an empty key, and the full form carrying `properties`.
fn palette_entries(
    chunk: &Chunk,
    y: i8,
    field: &'static str,
    compound: &BTreeMap<String, NbtTag>,
) -> Result<Vec<StoredEntry>, WireError> {
    let wrong = |detail: &str| chunk.wrong_type(&format!("section Y={y} {field}.palette {detail}"));
    let Some(NbtTag::List(items)) = compound.get("palette") else {
        return Err(wrong("is missing"));
    };
    let mut out = Vec::with_capacity(items.len());
    for tag in items {
        match tag {
            NbtTag::String(name) => out.push(StoredEntry {
                name: name.clone(),
                properties: None,
            }),
            NbtTag::Compound(fields) => {
                let name = ["id", "Name", ""]
                    .iter()
                    .find_map(|key| match fields.get(*key) {
                        Some(NbtTag::String(name)) => Some(name.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| wrong("holds no name"))?;
                out.push(StoredEntry {
                    name,
                    properties: match fields.get("properties") {
                        None => None,
                        Some(NbtTag::Compound(values)) => Some(
                            values
                                .iter()
                                .map(|(key, value)| match value {
                                    NbtTag::String(value) => Ok((key.clone(), value.clone())),
                                    _ => Err(wrong("holds a non-string property")),
                                })
                                .collect::<Result<BTreeMap<_, _>, WireError>>()?,
                        ),
                        Some(_) => return Err(wrong("holds non-compound properties")),
                    },
                });
            }
            _ => return Err(wrong("holds a non-string entry")),
        }
    }
    Ok(out)
}

/// Bits per cell of a container with `count` entries and the given floor.
fn cell_bits(count: usize, min_bits: usize) -> usize {
    let mut bits = 1;
    while bits < usize::BITS as usize && (1_usize << bits) < count {
        bits += 1;
    }
    bits.max(min_bits)
}

/// Longs `cells` cells occupy at `bits` per cell.
fn longs_for(cells: usize, bits: usize) -> usize {
    cells.div_ceil(64 / bits)
}

/// Decodes packed cells: one word holds `64 / bits` cells, least
/// significant bits first; the words are read as unsigned bits, so a cell
/// in the top bits of a negative long decodes correctly.
fn unpack(words: &[i64], bits: usize, cells: usize) -> Result<Vec<usize>, WireError> {
    let per_long = 64 / bits;
    let mask = (1_u64 << bits) - 1;
    Ok((0..cells)
        .map(|index| {
            let word = words[index / per_long] as u64;
            ((word >> ((index % per_long) * bits)) & mask) as usize
        })
        .collect())
}

/// Encodes cells back into big-endian longs; with the same bits this is
/// byte-identical to the stored words on the wire.
fn pack(cells: &[usize], bits: usize) -> Vec<u8> {
    let per_long = 64 / bits;
    let mut words = vec![0_u64; cells.len().div_ceil(per_long)];
    for (index, &value) in cells.iter().enumerate() {
        words[index / per_long] |= (value as u64) << ((index % per_long) * bits);
    }
    words.iter().flat_map(|word| word.to_be_bytes()).collect()
}

/// The two count headers of a section: cells that are not air and cells of a
/// fluid block.
///
/// SOURCE: parity trace — `non_air = 4096 − (air | cave_air | void_air)`
/// and the fluid count equal every `:water`/`:lava` block, proven on all
/// packed sections of the recorded full chunk; waterlogged variants of other
/// blocks were never observed and are not counted (docs/SOURCES.md).
fn counts(names: &[String], cells: &[usize]) -> (u16, u16) {
    const AIR: [&str; 3] = ["minecraft:air", "minecraft:cave_air", "minecraft:void_air"];
    let mut non_air = 0_usize;
    let mut fluid = 0_usize;
    for index in cells {
        let name = names[*index].as_str();
        if !AIR.contains(&name) {
            non_air += 1;
        }
        if name.ends_with(":water") || name.ends_with(":lava") {
            fluid += 1;
        }
    }
    (non_air as u16, fluid as u16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire_fixture::{self, TestRefs};

    #[test]
    fn bit_counts_are_the_measured_formula() {
        assert_eq!(cell_bits(1, 4), 4, "block floor of 4 bits");
        assert_eq!(cell_bits(16, 4), 4);
        assert_eq!(cell_bits(17, 4), 5, "342 longs = 5 bits");
        assert_eq!(cell_bits(33, 4), 6, "410 longs = 6 bits");
        assert_eq!(cell_bits(2, 1), 1, "biome pairs pack into one long");
        assert_eq!(cell_bits(3, 1), 2);
        assert_eq!(longs_for(4096, 4), 256, "the stored 256-long sections");
        assert_eq!(longs_for(4096, 5), 342);
        assert_eq!(longs_for(64, 1), 1, "64 biome cells at one bit per long");
    }

    #[test]
    fn packed_cells_roundtrip_and_the_first_word_is_pinned() {
        let cells: Vec<usize> = (0..BLOCK_CELLS).map(|index| index % 3).collect();
        let bytes = pack(&cells, 4);
        assert_eq!(bytes.len(), longs_for(BLOCK_CELLS, 4) * 8);
        // First long: nibble = cell % 3, least significant cell first —
        // 0x0210210210210210, written big-endian on the wire.
        assert_eq!(
            &bytes[0..8],
            &[0x02, 0x10, 0x21, 0x02, 0x10, 0x21, 0x02, 0x10]
        );
        let words: Vec<i64> = bytes
            .chunks(8)
            .map(|word| i64::from_be_bytes(word.try_into().expect("8 bytes")))
            .collect();
        assert_eq!(
            unpack(&words, 4, BLOCK_CELLS).expect("decodable"),
            cells,
            "decode inverts encode"
        );
        // A stored long whose top nibble is 15 (`-1` as i64) must still give
        // sixteen unsigned cells: the mask, not the sign, selects the bits.
        assert_eq!(
            unpack(&[-1_i64], 4, 16).expect("decodable"),
            vec![15; 16],
            "a cell of the negative long is read unsigned"
        );
    }

    #[test]
    fn a_packed_section_counts_air_and_fluid_from_the_palette() {
        let chunk = wire_fixture::packed_chunk();
        let sections = prepare(&chunk, &TestRefs).expect("fixture converts");
        let bottom = &sections[0];
        assert_eq!(bottom.record.non_air, 2, "stone and water are not air");
        assert_eq!(bottom.record.fluid, 1, "one water cell");
        assert!(matches!(
            &bottom.record.blocks,
            PaletteContainer::Packed { bits: 4, .. }
        ));
        match &bottom.record.biomes {
            PaletteContainer::Packed {
                bits,
                palette,
                data,
            } => {
                assert_eq!(*bits, 1, "two biomes at one bit each");
                assert_eq!(palette, &vec![1, 2]);
                assert_eq!(data.len(), 8, "64 cells at one bit = one long");
            }
            other => panic!("expected a packed biome container, got {other:?}"),
        }
    }

    #[test]
    fn a_section_the_template_stores_in_every_slot_is_required() {
        let mut chunk = wire_fixture::chunk();
        chunk.sections.retain(|section| section.y != -4);
        let error = prepare(&chunk, &TestRefs).expect_err("slot gone");
        assert!(
            matches!(error, WireError::MissingSection { y: -4, .. }),
            "{error}"
        );
    }

    #[test]
    fn a_palette_entry_unknown_to_the_reports_is_named() {
        let mut chunk = wire_fixture::packed_chunk();
        add_to_bottom_palette(&mut chunk, "minecraft:unobtainium");
        let error = prepare(&chunk, &TestRefs).expect_err("unknown block");
        assert!(
            matches!(
                error,
                WireError::UnknownBlock { ref name, .. } if name == "minecraft:unobtainium"
            ),
            "{error}"
        );
    }

    /// Puts an unresolvable block name into the bottom section's palette.
    fn add_to_bottom_palette(chunk: &mut Chunk, name: &str) {
        let section = &mut chunk.sections[0];
        if let Some(NbtTag::Compound(states)) = &mut section.block_states {
            if let Some(NbtTag::List(palette)) = states.get_mut("palette") {
                palette.push(NbtTag::String(name.to_owned()));
            }
        }
    }

    #[test]
    fn stored_data_of_the_wrong_length_is_reported_with_the_formula() {
        let mut chunk = wire_fixture::packed_chunk();
        if let Some(NbtTag::Compound(states)) = &mut chunk.sections[0].block_states {
            states.insert("data".to_owned(), NbtTag::LongArray(vec![0; 7]));
        }
        let error = prepare(&chunk, &TestRefs).expect_err("7 longs instead of 256");
        assert!(
            matches!(
                error,
                WireError::PackedLength {
                    longs: 7,
                    bits: 4,
                    expected: 256,
                    ..
                }
            ),
            "{error}"
        );
    }
}
