//! Step 3.2/4 (owner check 1): walking every section record of the wire
//! payload and proving that the sections blob is consumed exactly.
//!
//! `cargo test -p parity --test blob_walk -- --ignored --nocapture`
//!
//! The stored chunk holds twenty-four sections (Y −4..19); the wire record
//! of a section is
//! `[u16 non-air][u16 fluid][blocks container][biomes container]` where a
//! container is `[VarInt bits = 0][VarInt id]` for a single-valued
//! container and `[VarInt count][count × VarInt id][data]` otherwise (the
//! data length follows from `bits`: `ceil(cells / (64 / bits))` longs).
//! Walking all twenty-four records must land exactly on the end of the
//! blob declared at byte 903: overshooting or stopping early means the
//! framing is wrong.

#[path = "common/chunk_trace.rs"]
mod chunk_trace;

use std::path::Path;

use chunk_trace::{be_u16, palette_names, read_varint, vanilla_payload};
use protocol::NbtTag;

/// Byte holding the VarInt length of the sections blob.
const BLOB_LEN_AT: usize = 903;

/// One wire container (blocks or biomes) as read.
struct Container {
    /// Offset right after this container.
    end: usize,
    /// Bits per cell; 0 is the single-valued form.
    bits: i32,
    /// The single value when `bits` is 0.
    value: Option<i32>,
    /// Palette ids when `bits` is not 0.
    ids: Vec<i32>,
    /// Byte length of the packed data (0 when single-valued).
    data_len: usize,
}

/// Reads one container starting at `at`: a single value when `bits` is 0,
/// otherwise the palette ids and the packed data of `cells` cells.
fn read_container(payload: &[u8], at: usize, cells: usize) -> Option<Container> {
    let (bits, taken) = read_varint(payload, at)?;
    let mut cursor = at + taken;
    if bits == 0 {
        let (value, len) = read_varint(payload, cursor)?;
        return Some(Container {
            end: cursor + len,
            bits,
            value: Some(value),
            ids: Vec::new(),
            data_len: 0,
        });
    }
    if !(1..=16).contains(&bits) {
        return None;
    }
    let (count, len) = read_varint(payload, cursor)?;
    if count < 0 || count as usize > cells {
        return None;
    }
    cursor += len;
    let mut ids = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let (id, id_len) = read_varint(payload, cursor)?;
        ids.push(id);
        cursor += id_len;
    }
    let data_len = cells.div_ceil(64 / bits as usize) * 8;
    cursor += data_len;
    let end = cursor;
    Some(Container {
        end,
        bits,
        value: None,
        ids,
        data_len,
    })
}

/// `data` child of a stored section, as the count of packed bytes.
fn stored_data_len(section: &world::Section) -> usize {
    let Some(NbtTag::Compound(states)) = &section.block_states else {
        return 0;
    };
    match states.get("data") {
        Some(NbtTag::LongArray(values)) => values.len() * 8,
        _ => 0,
    }
}

/// How many of the 4096 packed cells of one section data array hold the
/// palette entry `wanted`: one long holds `64 / bits` cells, read from the
/// low bits up, the longs big-endian as on the wire.
fn count_cells(data: &[u8], bits: usize, wanted: usize) -> usize {
    let per_long = 64 / bits;
    let mask = (1_i64 << bits) - 1;
    let mut count = 0_usize;
    for cell in 0..4096_usize {
        let long_start = (cell / per_long) * 8;
        let Some(word) = data.get(long_start..long_start + 8) else {
            break;
        };
        let word = i64::from_be_bytes(word.try_into().expect("long of 8 bytes"));
        if (word >> ((cell % per_long) * bits)) & mask == wanted as i64 {
            count += 1;
        }
    }
    count
}

#[test]
#[ignore = "requires the owner's world template and the recorded trace"]
fn twenty_four_records_consume_the_blob_exactly() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let env = parity::load_parity_env().expect("parity environment");
    let chunk = chunk_trace::stored_chunk(&env.world_template);
    let payload = vanilla_payload(&workspace);

    assert_eq!(chunk.sections.len(), 24, "the template stores 24 sections");
    assert_eq!(chunk.sections[0].y, -4, "bottom section Y");
    assert_eq!(chunk.sections[23].y, 19, "top section Y");

    let (blob_len, taken) = read_varint(&payload, BLOB_LEN_AT).expect("blob length VarInt");
    assert_eq!(blob_len, 32581, "blob length of chunk (6, -2)");
    let mut cursor = BLOB_LEN_AT + taken;
    let end = cursor + blob_len as usize;

    let mut packed = 0_usize;
    for section in &chunk.sections {
        let non_air = be_u16(&payload, cursor).expect("u16 non-air");
        let fluid = be_u16(&payload, cursor + 2).expect("u16 fluid");
        let blocks = read_container(&payload, cursor + 4, 4096).expect("blocks container");
        let biomes = read_container(&payload, blocks.end, 64).expect("biomes container");
        let data = &payload[blocks.end - blocks.data_len..blocks.end];
        cursor = biomes.end;

        // The data the file stores must be exactly the data the wire sends;
        // a single-valued section must send no data at all.
        let stored = stored_data_len(section);
        assert_eq!(
            blocks.data_len, stored,
            "section Y={}: wire {} B of block data, NBT {stored} B",
            section.y, blocks.data_len
        );
        if stored > 0 {
            packed += 1;
        }

        // The header counts non-air cells: for a single-valued section the
        // palette says which block every cell holds, and the two headers
        // are what the rule counts.
        let names = palette_names(section);
        let single = if stored == 0 {
            names.first().cloned()
        } else {
            None
        };
        if let Some(name) = &single {
            assert!(blocks.value.is_some(), "Y={} single-valued", section.y);
            assert_eq!(
                non_air,
                if name == "minecraft:air" { 0 } else { 4096 },
                "section Y={} single-valued {name}",
                section.y
            );
        } else {
            assert_eq!(blocks.value, None, "Y={} packed section", section.y);
            assert!(
                non_air <= 4096 && non_air > 0,
                "Y={} header count",
                section.y
            );
        }
        // The second header counts fluid cells; the only fluid the data
        // carries here is `minecraft:water` (`minecraft:lava` counted the
        // same way when it appears). Whether the header means «water» or
        // «any fluid» cannot be decided on this chunk alone — one point.
        let mut expected = 0_usize;
        for name in ["minecraft:water", "minecraft:lava"] {
            let Some(index) = names.iter().position(|entry| entry == name) else {
                continue;
            };
            expected += if blocks.value.is_some() {
                4096
            } else {
                count_cells(data, blocks.bits as usize, index)
            };
        }
        assert_eq!(
            fluid,
            u16::try_from(expected).unwrap_or(u16::MAX),
            "section Y={} fluid header",
            section.y
        );
        eprintln!(
            "Y={:>3}: non-air {non_air}, fluid {fluid}, blocks: {} bits {} ({} B data), biomes: {}",
            section.y,
            blocks.bits,
            blocks.value.map_or_else(
                || format!("{} ids", blocks.ids.len()),
                |id| format!("single id {id}")
            ),
            blocks.data_len,
            biomes.bits
        );
    }

    eprintln!(
        "--- {packed} packed sections walked: cursor {cursor}, blob end {end} (delta {})",
        cursor as i64 - end as i64
    );
    assert_eq!(packed, 14, "the wire packs fourteen sections");
    assert_eq!(
        cursor, end,
        "all 24 records must consume the blob exactly ({cursor} vs {end})"
    );
}
