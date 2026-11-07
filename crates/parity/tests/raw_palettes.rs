//! Step 3.2: the raw `palette` lists of the stored chunk, read straight from
//! the region file instead of the parsed model.
//!
//! `cargo test -p parity --test raw_palettes -- --ignored --nocapture`
//!
//! Every `block_states`/`biomes` compound of the template chunk declares its
//! list type and count in the file itself. This test prints those declarations
//! and the first bytes of each list, so the parsed model used by the section
//! framing in `section_map.rs` can be checked against the file, not against
//! the parser.

/// Chunk coordinates of the first batch after the spawn fix (step 3.0/4).
const FIRST: (i32, i32) = (6, -2);

/// Decompresses a region sector by its compression byte (1 gzip, 2 zlib, 3 raw).
fn decompress(kind: u8, data: &[u8]) -> Vec<u8> {
    use std::io::Read;
    let mut out = Vec::new();
    let result = match kind {
        1 => flate2::read::GzDecoder::new(data).read_to_end(&mut out),
        2 => flate2::read::ZlibDecoder::new(data).read_to_end(&mut out),
        3 => {
            out.clear();
            out.extend_from_slice(data);
            Ok(0)
        }
        other => panic!("unknown region compression {other}"),
    };
    result.expect("decompression");
    out
}

/// Decompressed disk NBT of `FIRST`, read straight from the template's
/// region file (the parsed model may already have lost information; this
/// prints what the file actually holds).
fn raw_chunk_nbt() -> Vec<u8> {
    let env = parity::load_parity_env().expect("parity environment");
    let path = env
        .world_template
        .join("dimensions/minecraft/overworld/region/r.0.-1.mca");
    let file = std::fs::read(&path).expect("region file");
    let index = ((FIRST.1 + 32) as usize) * 32 + FIRST.0 as usize;
    let entry = &file[index * 4..index * 4 + 4];
    assert_ne!(entry[3], 0, "chunk {FIRST:?} slot is empty");
    let sector =
        (usize::from(entry[0]) << 16) | (usize::from(entry[1]) << 8) | usize::from(entry[2]);
    let offset = sector * 4096;
    let length = u32::from_be_bytes([
        file[offset],
        file[offset + 1],
        file[offset + 2],
        file[offset + 3],
    ]) as usize;
    let body = &file[offset + 5..offset + 4 + length];
    decompress(file[offset + 4], body)
}

/// Dumps every `palette` list of the stored chunk from the raw NBT bytes:
/// the element type and the count the file declares, and the first entries
/// as bytes. This is the ground truth the parsed model must match.
#[test]
#[ignore = "requires the owner's world template"]
fn raw_nbt_palette_lists_declare_their_own_counts() {
    let nbt = raw_chunk_nbt();
    let needle = b"palette";
    eprintln!(
        "--- palette keys in the raw chunk NBT ({} bytes) ---",
        nbt.len()
    );
    for start in 0..=nbt.len() - needle.len() {
        if &nbt[start..start + needle.len()] != needle {
            continue;
        }
        // The key stands inside a TAG_Compound: the tag type and the u16
        // name length precede it.
        let Some(tag) = start.checked_sub(3) else {
            continue;
        };
        if nbt[tag] != 9 {
            continue;
        }
        let element_type = nbt[start + needle.len()];
        let count = i32::from_be_bytes([
            nbt[start + needle.len() + 1],
            nbt[start + needle.len() + 2],
            nbt[start + needle.len() + 3],
            nbt[start + needle.len() + 4],
        ]);
        let from = start + needle.len() + 5;
        eprintln!("palette @ {start}: list TAG {tag}; element TAG {element_type}, count {count}");
        eprintln!(
            "  first entries: {:02x?}",
            &nbt[from..(from + 96).min(nbt.len())]
        );
    }
}
