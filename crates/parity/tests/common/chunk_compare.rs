//! Field-by-field comparison of one recorded `level_chunk_with_light`
//! against the own payload of the same coordinates (step 3.3f).
//!
//! Included by `chunk_slice.rs` through `#[path]`; every tallied group is
//! listed in `common::super::FIELDS`, i.e. the [`crate::FIELDS`] table of
//! the slice, and each group keeps the first differing example instead of
//! diffing everything twice.

use super::{record, Fields, FIELDS};
use protocol::{set_bits, LevelChunkWithLight};

/// One recorded chunk against the matching own payload, field by field,
/// tallied under [`FIELDS`].
pub(super) fn compare_pair(
    vanilla: &LevelChunkWithLight,
    ours: &LevelChunkWithLight,
    fields: &mut Fields,
) {
    let coord = (vanilla.chunk_x, vanilla.chunk_z);
    compare_heightmaps(&vanilla.heightmaps, &ours.heightmaps, coord, fields);
    compare_sections(&vanilla.sections, &ours.sections, coord, fields);
    compare_entities(&vanilla.block_entities, &ours.block_entities, coord, fields);
    compare_light(&vanilla.light, &ours.light, coord, fields);
}

/// Heightmap group: the ids (their order is vanilla's hash map iteration)
/// and the words of every map, compared id by id.
fn compare_heightmaps(
    vanilla: &[protocol::HeightmapRecord],
    ours: &[protocol::HeightmapRecord],
    coord: (i32, i32),
    fields: &mut Fields,
) {
    let id_list = |maps: &[protocol::HeightmapRecord]| {
        let ids: Vec<i32> = maps.iter().map(|map| map.id).collect();
        format!("{ids:?}")
    };
    record(
        fields,
        FIELDS[0],
        coord,
        id_list(vanilla) == id_list(ours),
        format!("vanilla {}, ours {}", id_list(vanilla), id_list(ours)),
    );
    let by_id = |maps: &[protocol::HeightmapRecord]| {
        let mut pairs: Vec<(i32, &Vec<i64>)> =
            maps.iter().map(|map| (map.id, &map.words)).collect();
        pairs.sort_by_key(|(id, _)| *id);
        pairs
            .iter()
            .map(|(id, words)| (*id, words.len()))
            .collect::<Vec<(i32, usize)>>()
    };
    let (v_shape, o_shape) = (by_id(vanilla), by_id(ours));
    let (v_words, o_words) = (heightmap_words(vanilla), heightmap_words(ours));
    let equal = v_words == o_words;
    record(
        fields,
        FIELDS[1],
        coord,
        equal,
        words_diff_text(equal, &v_shape, &o_shape, &v_words, &o_words),
    );
}

/// Where two heightmap groups differ: the shape first (`id → long count`),
/// then the map and the long that disagree.
fn words_diff_text(
    equal: bool,
    v_shape: &[(i32, usize)],
    o_shape: &[(i32, usize)],
    vanilla: &[(i32, &Vec<i64>)],
    ours: &[(i32, &Vec<i64>)],
) -> String {
    if equal {
        return "equal".to_owned();
    }
    if v_shape != o_shape {
        return format!("vanilla {v_shape:?}, ours {o_shape:?}");
    }
    for ((id, v), (_, o)) in vanilla.iter().zip(ours.iter()) {
        if v != o {
            let index = v
                .iter()
                .zip(o.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            return format!(
                "id {id}: long #{index} of {}: {} vanilla vs {} ours",
                v.len(),
                v.get(index).map_or("—".to_owned(), |word| word.to_string()),
                o.get(index).map_or("—".to_owned(), |word| word.to_string()),
            );
        }
    }
    "compare by id pair".to_owned()
}

/// Maps of a chunk as `id → words`, ordered by id so two chunks compare
/// without depending on the wire order vanilla happened to send.
fn heightmap_words(maps: &[protocol::HeightmapRecord]) -> Vec<(i32, &Vec<i64>)> {
    let mut pairs: Vec<(i32, &Vec<i64>)> = maps.iter().map(|map| (map.id, &map.words)).collect();
    pairs.sort_by_key(|(id, _)| *id);
    pairs
}

/// Section group: the slot count, both headers and both containers of
/// every section in pack order (bottom to top).
fn compare_sections(
    vanilla: &[protocol::ChunkSection],
    ours: &[protocol::ChunkSection],
    coord: (i32, i32),
    fields: &mut Fields,
) {
    record(
        fields,
        FIELDS[2],
        coord,
        vanilla.len() == ours.len(),
        format!("{} vanilla vs ours", vanilla.len()),
    );
    let pairs: Vec<_> = vanilla.iter().zip(ours.iter()).collect();
    let header_first = pairs
        .iter()
        .position(|(v, o)| (v.non_air, v.fluid) != (o.non_air, o.fluid));
    record(
        fields,
        FIELDS[3],
        coord,
        header_first.is_none(),
        section_first(header_first, &pairs, |v, o| {
            format!(
                "non-air {:?} vs {:?}",
                (v.non_air, v.fluid),
                (o.non_air, o.fluid)
            )
        }),
    );
    compare_containers(fields, FIELDS[4], coord, &pairs, |section| &section.blocks);
    compare_containers(fields, FIELDS[5], coord, &pairs, |section| &section.biomes);
}

/// Compares one container of every section pair and tallies the field.
fn compare_containers(
    fields: &mut Fields,
    field: &'static str,
    coord: (i32, i32),
    pairs: &[(&protocol::ChunkSection, &protocol::ChunkSection)],
    pick: fn(&protocol::ChunkSection) -> &protocol::PaletteContainer,
) {
    let first = pairs.iter().position(|(v, o)| pick(v) != pick(o));
    record(
        fields,
        field,
        coord,
        first.is_none(),
        section_first(first, pairs, |v, o| container_diff(pick(v), pick(o))),
    )
}

/// What exactly differs between two containers: the palette ids, the
/// packed words, or a change of form between them.
fn container_diff(
    vanilla: &protocol::PaletteContainer,
    ours: &protocol::PaletteContainer,
) -> String {
    let base = format!("{} vs {}", container_text(vanilla), container_text(ours));
    use protocol::PaletteContainer as C;
    let (
        C::Packed {
            palette: v_pal,
            data: v_data,
            ..
        },
        C::Packed {
            palette: o_pal,
            data: o_data,
            ..
        },
    ) = (vanilla, ours)
    else {
        return base;
    };
    let palette = if v_pal == o_pal {
        "palette equal".to_owned()
    } else {
        let index = v_pal
            .iter()
            .zip(o_pal.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        format!(
            "palette differs at #{} of {}/{}: {} vanilla vs {}",
            index,
            v_pal.len(),
            o_pal.len(),
            v_pal.get(index).map_or("—".to_owned(), |id| id.to_string()),
            o_pal.get(index).map_or("—".to_owned(), |id| id.to_string()),
        )
    };
    let data = if v_data == o_data {
        "data equal".to_owned()
    } else if v_data.len() != o_data.len() {
        format!("data {} vs {} bytes", v_data.len(), o_data.len())
    } else {
        let byte = v_data
            .iter()
            .zip(o_data.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        format!(
            "data differs at byte {byte}: 0x{:02X} vanilla vs 0x{:02X} ours",
            v_data[byte], o_data[byte]
        )
    };
    format!("{palette}; {data}")
}

/// `section #i: <two values>` of the first differing pair of sections.
fn section_first<F>(
    first: Option<usize>,
    pairs: &[(&protocol::ChunkSection, &protocol::ChunkSection)],
    show: F,
) -> String
where
    F: Fn(&protocol::ChunkSection, &protocol::ChunkSection) -> String,
{
    match first {
        Some(index) => match pairs.get(index) {
            Some((v, o)) => format!("section #{index}: {}", show(v, o)),
            None => "more sections on one side".to_owned(),
        },
        None => "equal".to_owned(),
    }
}

/// One palette container as the table describes it.
fn container_text(container: &protocol::PaletteContainer) -> String {
    match container {
        protocol::PaletteContainer::Single(value) => format!("single {value}"),
        protocol::PaletteContainer::Packed {
            bits,
            palette,
            data,
        } => format!(
            "{bits} bits, {} palette, {} bytes",
            palette.len(),
            data.len()
        ),
    }
}

/// Block entity group: the count, the records in the sent order (`xz`,
/// `y`, type — every field but the NBT) and the NBT bytes themselves.
fn compare_entities(
    vanilla: &[protocol::BlockEntity],
    ours: &[protocol::BlockEntity],
    coord: (i32, i32),
    fields: &mut Fields,
) {
    record(
        fields,
        FIELDS[6],
        coord,
        vanilla.len() == ours.len(),
        format!("{} vanilla vs ours", vanilla.len()),
    );
    let shape = |entities: &[protocol::BlockEntity]| {
        entities
            .iter()
            .map(|entity| (entity.local_x, entity.local_z, entity.y, entity.type_id))
            .collect::<Vec<_>>()
    };
    let (v_shape, o_shape) = (shape(vanilla), shape(ours));
    record(
        fields,
        FIELDS[7],
        coord,
        v_shape == o_shape,
        format!("vanilla {}, ours {}", short(&v_shape), short(&o_shape)),
    );
    let mut nbt_equal = vanilla.len() == ours.len();
    let mut first_diff = None;
    for (index, (v, o)) in vanilla.iter().zip(ours.iter()).enumerate() {
        if v.nbt != o.nbt {
            nbt_equal = false;
            first_diff = Some(format!(
                "record #{index}: vanilla {} bytes, ours {} (chunk local {}, {})",
                v.nbt.len(),
                o.nbt.len(),
                v.local_x,
                v.local_z
            ));
            break;
        }
    }
    // A count mismatch alone still needs a name: without it the row would
    // read "equal" while the tally below says otherwise.
    let detail = first_diff.unwrap_or_else(|| {
        if nbt_equal {
            "equal".to_owned()
        } else {
            format!(
                "record counts: {} vanilla vs {} ours",
                vanilla.len(),
                ours.len()
            )
        }
    });
    record(fields, FIELDS[8], coord, nbt_equal, detail);
}

/// First three records as a compact list (`…` when there are more).
fn short(records: &[(u8, u8, i16, i32)]) -> String {
    let head = &records[..records.len().min(3)];
    if records.len() > 3 {
        format!("{head:?}, …")
    } else {
        format!("{head:?}")
    }
}

/// Light group: four masks as the sent bits and both array groups.
fn compare_light(
    vanilla: &protocol::LightData,
    ours: &protocol::LightData,
    coord: (i32, i32),
    fields: &mut Fields,
) {
    for (field, (v_mask, o_mask)) in [
        (FIELDS[9], (&vanilla.sky_mask, &ours.sky_mask)),
        (FIELDS[10], (&vanilla.block_mask, &ours.block_mask)),
        (FIELDS[11], (&vanilla.empty_sky_mask, &ours.empty_sky_mask)),
        (
            FIELDS[12],
            (&vanilla.empty_block_mask, &ours.empty_block_mask),
        ),
    ] {
        record(
            fields,
            field,
            coord,
            v_mask == o_mask,
            format!("vanilla {}, ours {}", bits_text(v_mask), bits_text(o_mask)),
        );
    }
    for (field, (v_arrays, o_arrays)) in [
        (FIELDS[13], (&vanilla.sky_arrays, &ours.sky_arrays)),
        (FIELDS[14], (&vanilla.block_arrays, &ours.block_arrays)),
    ] {
        record(
            fields,
            field,
            coord,
            v_arrays == o_arrays,
            arrays_text(v_arrays, o_arrays),
        );
    }
}

/// Set bits of one light mask as the table reads them (`[0, 1, …]`).
fn bits_text(mask: &[u8]) -> String {
    let bits = set_bits(mask);
    if bits.is_empty() {
        return "[]".to_owned();
    }
    if bits.len() > 14 {
        return format!("[{} bits: {:?}…]", bits.len(), &bits[..12]);
    }
    format!("{bits:?}")
}

/// Difference of two light array groups: the count mismatch, or the first
/// array and byte that disagree.
fn arrays_text(vanilla: &[Vec<u8>], ours: &[Vec<u8>]) -> String {
    if vanilla.len() != ours.len() {
        return format!("{} vanilla vs {} ours", vanilla.len(), ours.len());
    }
    for (index, (v, o)) in vanilla.iter().zip(ours.iter()).enumerate() {
        if let Some(byte) = v.iter().zip(o.iter()).position(|(a, b)| a != b) {
            return format!(
                "{} arrays, first difference in #{index} byte {byte}: 0x{:02X} vanilla vs 0x{:02X} ours",
                vanilla.len(),
                v[byte],
                o[byte]
            );
        }
    }
    "byte equal".to_owned()
}
