//! The send gate: from chunk coordinates to a chunk the ready rule allows
//! out of the world, with an explicit reason when it does not.
//!
//! Everything on the send path goes through [`load_sendable_chunk`], so a
//! caller never has to remember the rule itself (see [`crate::chunk::Chunk::check_sendable`],
//! derived from the owner's template and join recording, step 3.0): a chunk
//! comes back either fully sendable or refused by name — there is no third
//! outcome, and no caller falls back to an empty or invented chunk.

use std::path::Path;

use crate::chunk::{Chunk, ChunkNotReady};
use crate::error::WorldError;
use crate::region::load_region;

/// Why [`load_sendable_chunk`] did not hand out a chunk.
///
/// The struct field names follow the tag: a refusal always says which
/// chunk and which piece of data was responsible.
#[derive(Debug, thiserror::Error)]
pub enum ChunkUnavailable {
    /// The chunk is stored but fails the ready rule. The loader keeps the
    /// coordinates for the «waits for generation (M6)» list of
    /// `docs/PROGRESS.md` and says why.
    #[error(transparent)]
    NotReady(#[from] ChunkNotReady),
    /// No region file covers the coordinates, or its slot is empty: the
    /// template holds nothing to send there (chunk generation is M6).
    #[error("chunk ({x}, {z}) is not stored in the world")]
    NotStored {
        /// Chunk x that has no stored data.
        x: i32,
        /// Chunk z that has no stored data.
        z: i32,
    },
    /// The region file or the NBT inside it could not be read: a malformed
    /// world is a read error, never silently a missing chunk.
    #[error("chunk ({x}, {z}) cannot be read: {source}")]
    Unreadable {
        /// Chunk x whose region failed to read.
        x: i32,
        /// Chunk z whose region failed to read.
        z: i32,
        /// The read failure underneath.
        #[source]
        source: WorldError,
    },
}

/// Reads the chunk at `(x, z)` from `region_dir` and returns it only when
/// the full send rule accepts it.
///
/// `region_dir` is the `dimensions/<dimension>/region` directory of the
/// world being served; the region file itself follows the Anvil layout
/// `r.<x.div_32>.<z.div_32>.mca`. Every rejection is a named
/// [`ChunkUnavailable`] value: an absent chunk, a chunk waiting for
/// generation, a wrong `DataVersion` or a corrupt file stay distinguishable
/// for the caller.
pub fn load_sendable_chunk(
    region_dir: &Path,
    x: i32,
    z: i32,
    data_version: i32,
) -> Result<Chunk, ChunkUnavailable> {
    let file = region_dir.join(format!("r.{}.{}.mca", x.div_euclid(32), z.div_euclid(32)));
    if !file.is_file() {
        return Err(ChunkUnavailable::NotStored { x, z });
    }
    let stored = load_region(&file)
        .map_err(|source| ChunkUnavailable::Unreadable { x, z, source })?
        .into_iter()
        .find(|stored| stored.x == x && stored.z == z)
        .ok_or(ChunkUnavailable::NotStored { x, z })?;
    let chunk = Chunk::from_stored(&stored).map_err(|source| ChunkUnavailable::Unreadable {
        x,
        z,
        source,
    })?;
    chunk.check_sendable(data_version)?;
    Ok(chunk)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERSION: i32 = 5023;

    /// A temp directory of one test, empty and its own.
    fn region_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("mine_rust-world-send-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("fresh temp dir");
        dir
    }

    /// Disk NBT of a stored chunk: an empty root name, then a `DataVersion`
    /// int, a `Status` string and a `sections` list (empty, so the rule
    /// finds no light to send).
    fn disk_nbt(data_version: i32, status: &str) -> Vec<u8> {
        let mut out = vec![0x0A, 0x00, 0x00]; // compound, empty root name
        out.extend_from_slice(&[0x03, 0x00, 0x0B]); // int, name length 11
        out.extend_from_slice(b"DataVersion");
        out.extend_from_slice(&data_version.to_be_bytes());
        out.extend_from_slice(&[0x08, 0x00, 0x06]); // string, name length 6
        out.extend_from_slice(b"Status");
        out.extend_from_slice(&(status.len() as u16).to_be_bytes());
        out.extend_from_slice(status.as_bytes());
        out.extend_from_slice(&[0x09, 0x00, 0x08]); // list, name length 8
        out.extend_from_slice(b"sections");
        out.extend_from_slice(&[0x0A, 0x00, 0x00, 0x00, 0x00]); // no compounds
        out.push(0x00); // end of the root compound
        out
    }

    /// Writes one region file holding a single chunk at `(x, z)`: the two
    /// 4 KiB headers, then one length-prefixed zlib sector (the layout the
    /// reader was tested against in `region.rs`).
    fn store_chunk(dir: &Path, x: i32, z: i32, nbt: &[u8]) {
        use std::io::Write;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(nbt).expect("zlib compress");
        let payload = encoder.finish().expect("zlib finished");

        let slot = x.rem_euclid(32) as usize + 32 * z.rem_euclid(32) as usize;
        let sectors = (payload.len() + 5).div_ceil(4096) as u8;
        let mut file = vec![0_u8; 8192];
        // Offset in sectors: the payload starts right after both headers,
        // at byte 8192 = sector 2; the byte after the offset holds the count.
        file[slot * 4] = 0;
        file[slot * 4 + 1] = 0;
        file[slot * 4 + 2] = 2;
        file[slot * 4 + 3] = sectors;
        file.extend_from_slice(&(payload.len() as u32 + 1).to_be_bytes());
        file.push(2); // zlib
        file.extend_from_slice(&payload);
        std::fs::write(dir.join(format!("r.{}.mca", r_name(x, z))), &file).expect("region written");
    }

    /// Region file name of a chunk in the Anvil layout.
    fn r_name(x: i32, z: i32) -> String {
        format!("{}.{}", x.div_euclid(32), z.div_euclid(32))
    }

    #[test]
    fn coordinates_without_a_region_file_are_not_stored() {
        let dir = region_dir("far");
        let error = load_sendable_chunk(&dir, 40_000, 40_000, VERSION)
            .expect_err("no region file is that far out");
        assert!(
            matches!(
                error,
                ChunkUnavailable::NotStored {
                    x: 40_000,
                    z: 40_000
                }
            ),
            "{error}"
        );
        assert!(error.to_string().contains("40000"), "{error}");
    }

    #[test]
    fn an_empty_region_never_invents_a_chunk() {
        let dir = region_dir("empty");
        std::fs::write(dir.join("r.0.0.mca"), vec![0_u8; 8192]).expect("empty region");
        let error = load_sendable_chunk(&dir, 0, 0, VERSION).expect_err("every slot is empty");
        assert!(
            matches!(error, ChunkUnavailable::NotStored { x: 0, z: 0 }),
            "{error}"
        );
    }

    #[test]
    fn a_malformed_region_is_unreadable_not_missing() {
        let dir = region_dir("corrupt");
        std::fs::write(dir.join("r.0.0.mca"), [0_u8; 10]).expect("truncated file");
        let error = load_sendable_chunk(&dir, 0, 0, VERSION).expect_err("shorter than the header");
        match error {
            ChunkUnavailable::Unreadable { x, z, source } => {
                assert_eq!((x, z), (0, 0));
                assert!(
                    source.to_string().contains("8192") || source.to_string().contains("shorter"),
                    "{source}"
                );
            }
            other => panic!("expected Unreadable, got {other}"),
        }
    }

    #[test]
    fn an_ungenerated_chunk_names_its_status_through_the_gate() {
        let dir = region_dir("terrain");
        store_chunk(&dir, 0, 0, &disk_nbt(VERSION, "minecraft:terrain"));
        let error = load_sendable_chunk(&dir, 0, 0, VERSION).expect_err("terrain waits for M6");
        assert!(
            matches!(
                &error,
                ChunkUnavailable::NotReady(ChunkNotReady::NotGenerated { x: 0, z: 0, status })
                    if status == "minecraft:terrain"
            ),
            "{error}"
        );
    }

    #[test]
    fn a_ready_status_without_any_light_is_refused_by_name() {
        let dir = region_dir("dark");
        store_chunk(&dir, 0, 0, &disk_nbt(VERSION, "minecraft:full"));
        let error = load_sendable_chunk(&dir, 0, 0, VERSION).expect_err("no light to send");
        assert!(
            matches!(
                &error,
                ChunkUnavailable::NotReady(ChunkNotReady::NoLight { x: 0, z: 0 })
            ),
            "{error}"
        );
    }

    #[test]
    fn the_wrong_data_version_is_refused_before_anything_else() {
        let dir = region_dir("version");
        store_chunk(&dir, 0, 0, &disk_nbt(VERSION, "minecraft:full"));
        let error = load_sendable_chunk(&dir, 0, 0, 9999).expect_err("another schema");
        assert!(
            matches!(
                &error,
                ChunkUnavailable::NotReady(ChunkNotReady::UnsupportedVersion {
                    x: 0,
                    z: 0,
                    found: 5023,
                    expected: 9999,
                })
            ),
            "{error}"
        );
    }
}
