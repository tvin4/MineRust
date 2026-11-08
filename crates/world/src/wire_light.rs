//! The light block of the chunk payload.
//!
//! SOURCE: the decision of `docs/DECISIONS.md` (§141 «sections without
//! light»), which is deliberately a temporary rule and **not** parity with
//! the recording: a section whose stored NBT carries a `SkyLight` or
//! `BlockLight` array goes on the wire as that array; a section without
//! stored light is sent as sky 15 (one fully lit 2048-byte array, so every
//! section has its sky entry) and block 0 (the section's bit in the empty
//! mask, no array). The recorded light arrays and masks differ from the
//! stored template in all nine chunks — pinned as a known deviation by
//! `parity/light_framing` and reported, not guessed away, by the 3.3f test.
//!
//! The four masks index the sections bottom-up: section Y −4 is bit 0, so
//! the template's 24 slots are bits 0..23.
//! SOURCE: wiki, unverified — the recording's own masks stop at bits 15/16
//! (its light engine left the upper sections out), so the convention is
//! structural here and only the client run after 3.3 can confirm it
//! visually.

use protocol::{LightData, LIGHT_ARRAY_BYTES};

use crate::chunk::{Chunk, Section};
use crate::wire::WireError;
use crate::wire_section::SLOT_COUNT;

/// Bytes of one mask: 24 template slots = 3 bytes.
const MASK_BYTES: usize = 3;

/// Builds the light block of this ready-checked chunk.
pub(crate) fn light(chunk: &Chunk) -> Result<LightData, WireError> {
    let mut masks = (
        vec![0_u8; MASK_BYTES],
        vec![0_u8; MASK_BYTES],
        vec![0_u8; MASK_BYTES],
        vec![0_u8; MASK_BYTES],
    );
    let mut sky_arrays: Vec<(usize, Vec<u8>)> = Vec::with_capacity(SLOT_COUNT);
    let mut block_arrays: Vec<(usize, Vec<u8>)> = Vec::with_capacity(SLOT_COUNT);
    for section in by_height(chunk) {
        let bit = slot(chunk, section)?;
        let sky = stored(chunk, section, &section.sky_light, "SkyLight")?
            .unwrap_or_else(|| vec![0xFF; LIGHT_ARRAY_BYTES]);
        set_bit(&mut masks.0, bit);
        sky_arrays.push((bit, sky));
        match stored(chunk, section, &section.block_light, "BlockLight")? {
            Some(block) => {
                set_bit(&mut masks.1, bit);
                block_arrays.push((bit, block));
            }
            // §141 sends a synthesized sky array, so a dirt-less section is
            // never empty in the sky mask — only block light falls to zero.
            None => set_bit(&mut masks.3, bit),
        }
    }
    Ok(LightData {
        sky_mask: masks.0,
        block_mask: masks.1,
        empty_sky_mask: masks.2,
        empty_block_mask: masks.3,
        sky_arrays: in_bit_order(sky_arrays),
        block_arrays: in_bit_order(block_arrays),
    })
}

/// The sections placed in mask/bit order (bottom to top), regardless of how
/// the file lists them.
fn by_height(chunk: &Chunk) -> Vec<&Section> {
    let mut sections: Vec<&Section> = chunk.sections.iter().collect();
    sections.sort_by_key(|section| section.y);
    sections
}

/// The mask bit of one section: its slot index (Y −4 = 0).
fn slot(chunk: &Chunk, section: &Section) -> Result<usize, WireError> {
    usize::try_from(i32::from(section.y) + 4)
        .ok()
        .filter(|bit| *bit < SLOT_COUNT)
        .ok_or_else(|| {
            chunk.wrong_type(&format!(
                "section Y={} is outside the slots Y={}..Y={}",
                section.y,
                crate::wire_section::SLOT.0,
                crate::wire_section::SLOT.1
            ))
        })
}

/// One stored light array as wire bytes; `None` when the section stores no
/// array of that kind (the caller applies the §141 rule to it).
fn stored(
    chunk: &Chunk,
    section: &Section,
    bytes: &Option<Vec<i8>>,
    field: &'static str,
) -> Result<Option<Vec<u8>>, WireError> {
    let Some(bytes) = bytes else {
        return Ok(None);
    };
    if bytes.len() != LIGHT_ARRAY_BYTES {
        return Err(WireError::LightLength {
            x: chunk.x,
            z: chunk.z,
            y: section.y,
            field,
            bytes: bytes.len(),
        });
    }
    Ok(Some(bytes.iter().map(|byte| *byte as u8).collect()))
}

/// Sets one mask bit: byte `bit / 8`, branch `bit % 8` (Java BitSet order).
fn set_bit(mask: &mut [u8], bit: usize) {
    mask[bit / 8] |= 1_u8 << (bit % 8);
}

/// Collects arrays in ascending bit order, which is the order the masks
/// list them.
fn in_bit_order(mut arrays: Vec<(usize, Vec<u8>)>) -> Vec<Vec<u8>> {
    arrays.sort_by_key(|(bit, _)| *bit);
    arrays.into_iter().map(|(_, bytes)| bytes).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire_fixture;

    #[test]
    fn sections_without_stored_light_go_out_as_sky_15_and_block_0() {
        let light = light(&wire_fixture::chunk()).expect("fixture converts");
        assert_eq!(light.sky_mask, vec![0xFF; MASK_BYTES], "every slot set");
        assert_eq!(
            light.empty_block_mask,
            vec![0xFF; MASK_BYTES],
            "every slot zero block light"
        );
        assert_eq!(light.block_mask, vec![0_u8; MASK_BYTES], "no array");
        assert_eq!(
            light.empty_sky_mask,
            vec![0_u8; MASK_BYTES],
            "§141 sends sky arrays"
        );
        assert_eq!(light.sky_arrays.len(), wire_fixture::SECTION_SLOTS);
        assert!(light.block_arrays.is_empty());
        assert!(
            light
                .sky_arrays
                .iter()
                .all(|array| array == &vec![0xFF; LIGHT_ARRAY_BYTES]),
            "sky 15 in every array"
        );
    }

    #[test]
    fn stored_arrays_win_and_land_in_their_masks() {
        let mut chunk = wire_fixture::chunk();
        chunk.sections[0].sky_light = Some(vec![0x42_i8; LIGHT_ARRAY_BYTES]);
        chunk.sections[0].block_light = Some(vec![0x11_i8; LIGHT_ARRAY_BYTES]);
        let light = light(&chunk).expect("fixture converts");
        // §141 gives every section a sky array (stored or synthesized), so
        // every slot is occupied in the sky mask.
        assert_eq!(light.sky_mask, vec![0xFF; MASK_BYTES]);
        assert_eq!(light.block_mask, vec![0x01, 0x00, 0x00]);
        assert_eq!(
            light.empty_block_mask,
            vec![0xFE, 0xFF, 0xFF],
            "every other slot keeps block 0"
        );
        assert_eq!(light.sky_arrays.len(), wire_fixture::SECTION_SLOTS);
        assert_eq!(light.sky_arrays[0], vec![0x42_u8; LIGHT_ARRAY_BYTES]);
        assert_eq!(
            light.sky_arrays[1],
            vec![0xFF_u8; LIGHT_ARRAY_BYTES],
            "the rest is synthesized as §141 says"
        );
        assert_eq!(light.block_arrays, vec![vec![0x11_u8; LIGHT_ARRAY_BYTES]]);
    }

    #[test]
    fn a_stored_array_of_the_wrong_length_is_named() {
        let mut chunk = wire_fixture::chunk();
        chunk.sections[0].sky_light = Some(vec![0; 10]);
        let error = light(&chunk).expect_err("10 bytes instead of 2048");
        assert!(
            matches!(
                error,
                WireError::LightLength {
                    y: -4,
                    field: "SkyLight",
                    bytes: 10,
                    ..
                }
            ),
            "{error}"
        );
    }
}
