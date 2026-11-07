//! The light block of `level_chunk_with_light`, pinned by data (9 chunks).
//!
//! `cargo test -p parity --test light_framing -- --ignored --nocapture`
//!
//! After the block entities the wire carries light, framing derived from
//! the owner's trace (not from the wiki):
//!
//! `[sky mask BitSet][block mask BitSet][empty sky BitSet]
//!  [empty block BitSet][VarInt sky count][sky × (VarInt 2048 + 2048 B)]
//!  [VarInt block count][block × (VarInt 2048 + 2048 B)]`
//!
//! A BitSet is `[VarInt nbytes][nbytes]` where byte `i` holds bits
//! `8i..8i+7` (Java `BitSet.toByteArray` order): the four masks of every
//! chunk parse to disjoint section sets, each mask's bit count equals the
//! array count that follows it, and the last array lands exactly at the
//! end of the payload. Block entities before the light have variable NBT
//! (the spawner of chunk (5,-1) is 175 bytes), so their records are walked
//! with a real NBT reader, not with a fixed stride.

#[path = "common/chunk_trace.rs"]
mod chunk_trace;

use std::path::Path;

use chunk_trace::{all_payloads, read_varint};

/// Network NBT value length at `at` for a value of tag `tag`.
fn value_len(bytes: &[u8], at: usize, tag: u8) -> Option<usize> {
    match tag {
        0 => Some(0),
        1 => Some(1),
        2 => Some(2),
        3 | 5 => Some(4),
        4 | 6 => Some(8),
        7 => {
            let len = i32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?);
            Some(4 + usize::try_from(len).ok()?)
        }
        8 => {
            let len = u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?);
            Some(2 + usize::from(len))
        }
        9 => value_list_len(bytes, at),
        10 => compound_len(bytes, at),
        11 | 12 => {
            let width = if tag == 11 { 4 } else { 8 };
            let len = i32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?);
            Some(4 + usize::try_from(len).ok()? * width)
        }
        _ => None,
    }
}

/// Length of a `TAG_List` at `at`: element tag byte, count, then every
/// element (element length depends on the tag, so the elements are walked
/// one by one).
fn value_list_len(bytes: &[u8], at: usize) -> Option<usize> {
    let element = *bytes.get(at)?;
    let len = i32::from_be_bytes(bytes.get(at + 1..at + 5)?.try_into().ok()?);
    let mut cursor = at + 5;
    for _ in 0..usize::try_from(len).ok()? {
        let size = value_len(bytes, cursor, element)?;
        cursor += size;
    }
    Some(cursor - at)
}

/// Length of a compound payload at `at` (children, then the `00`
/// terminator); `at` points *past* the opening `0x0a` tag byte.
fn compound_len(bytes: &[u8], at: usize) -> Option<usize> {
    let mut cursor = at;
    while *bytes.get(cursor)? != 0 {
        let tag = *bytes.get(cursor)?;
        cursor += 1;
        let name = u16::from_be_bytes(bytes.get(cursor..cursor + 2)?.try_into().ok()?);
        cursor += 2 + usize::from(name);
        cursor += value_len(bytes, cursor, tag)?;
    }
    Some(cursor + 1 - at)
}

/// Length of a whole record NBT: one `00` byte (empty root) or the `0x0a`
/// root tag plus its payload.
fn nbt_len(bytes: &[u8], at: usize) -> Option<usize> {
    match *bytes.get(at)? {
        0x00 => Some(1),
        0x0a => Some(1 + compound_len(bytes, at + 1)?),
        _ => None,
    }
}

/// Block entity records starting at `at`: how many there are, `(type id,
/// nbt length)` of each, and the offset right after the last record — the
/// first light byte.
fn block_entities(bytes: &[u8], at: usize) -> (Vec<(i32, usize)>, usize) {
    let (count, taken) = read_varint(bytes, at).expect("entity count VarInt");
    let mut cursor = at + taken;
    let mut out = Vec::new();
    for _ in 0..count {
        cursor += 3; // packed xz (u8) + y (i16 BE)
        let (id, taken) = read_varint(bytes, cursor).expect("type id VarInt");
        cursor += taken;
        let nbt = nbt_len(bytes, cursor).expect("network NBT record");
        cursor += nbt;
        out.push((id, nbt));
    }
    (out, cursor)
}

/// A BitSet read at `at`: `[VarInt nbytes][nbytes]`, byte `i` holding the
/// bits `8i..8i+7`. Returns the set bits and the offset after it.
fn bitset(bytes: &[u8], at: usize) -> (Vec<usize>, usize) {
    let (size, taken) = read_varint(bytes, at).expect("BitSet size VarInt");
    let start = at + taken;
    let mut bits = Vec::new();
    for index in 0..size as usize {
        let byte = *bytes.get(start + index).expect("BitSet byte");
        for branch in 0..8_usize {
            if byte >> branch & 1 != 0 {
                bits.push(index * 8 + branch);
            }
        }
    }
    (bits, start + size as usize)
}

/// One group of light arrays after its VarInt count: every array is
/// `[VarInt 2048][2048 bytes]`. Returns `(count, offset after)`.
fn light_arrays(bytes: &[u8], at: usize) -> (i32, usize) {
    let (count, taken) = read_varint(bytes, at).expect("array count VarInt");
    let mut cursor = at + taken;
    for _ in 0..count {
        let (len, taken) = read_varint(bytes, cursor).expect("array length VarInt");
        assert_eq!(len, 2048, "one nibble per block: 4096 cells");
        cursor += taken + len as usize;
    }
    (count, cursor)
}

/// The four masks, both array counts and the end offset of one payload's
/// light block (everything between the block entities and the payload end).
fn light_block(bytes: &[u8]) -> (Vec<Vec<usize>>, Vec<i32>, usize) {
    let (blob, taken) = read_varint(bytes, 903).expect("blob length VarInt");
    let entities_at = 903 + taken + blob as usize;
    let (records, light_at) = block_entities(bytes, entities_at);
    eprintln!("entities: {records:?} (light at {light_at})");
    let (sky, cursor) = bitset(bytes, light_at);
    let (block, cursor) = bitset(bytes, cursor);
    let (empty_sky, cursor) = bitset(bytes, cursor);
    let (empty_block, cursor) = bitset(bytes, cursor);
    let (sky_count, cursor) = light_arrays(bytes, cursor);
    let (block_count, cursor) = light_arrays(bytes, cursor);
    (
        vec![sky, block, empty_sky, empty_block],
        vec![sky_count, block_count],
        cursor,
    )
}

#[test]
#[ignore = "requires the owner's files and the recorded trace"]
fn the_light_block_frames_all_nine_chunks() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut seen = 0_usize;
    for ((x, z), bytes) in all_payloads(&workspace) {
        let (masks, counts, end) = light_block(&bytes);
        let [sky, block, empty_sky, empty_block] = [&masks[0], &masks[1], &masks[2], &masks[3]];
        eprintln!(
            "({x}, {z})\n  sky {sky:?}\n  block {block:?}\n  \
             empty sky {empty_sky:?}\n  empty block {empty_block:?}"
        );
        // The mask pair must agree with the counts that follow it: one
        // array per set bit, no more, no less.
        assert_eq!(
            sky.len() as i32,
            counts[0],
            "chunk ({x}, {z}): sky mask vs count"
        );
        assert_eq!(
            block.len() as i32,
            counts[1],
            "chunk ({x}, {z}): block mask vs count"
        );
        // Each chain pairs up section by section: a section carries an
        // array of that chain or is marked empty in it, never both.
        for ((mask, empty), name) in [((sky, empty_sky), "sky"), ((block, empty_block), "block")] {
            let both: Vec<usize> = mask
                .iter()
                .filter(|bit| empty.contains(bit))
                .copied()
                .collect();
            assert!(
                both.is_empty(),
                "chunk ({x}, {z}): {name} mask {both:?} also marked empty"
            );
        }
        assert_eq!(
            end,
            bytes.len(),
            "chunk ({x}, {z}): light must end exactly at the payload end"
        );
        seen += 1;
    }
    assert_eq!(seen, 9, "nine chunks frame their light");
}
