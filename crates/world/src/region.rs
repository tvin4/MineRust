//! The Anvil region reader: `r.<x>.<z>.mca` files.
//!
//! Layout observed from the owner's template (public Anvil description
//! matches): a 4 KiB header of chunk locations (3-byte sector offset plus a
//! sector count) followed by a 4 KiB header of timestamps, then length-
//! prefixed sectors: `length (4 bytes, includes the type byte) + type byte +
//! payload`. Compression types 1/2/3 are gzip/zlib/raw respectively, all zlib
//! in the template.

use std::path::Path;

use protocol::NbtTag;

use crate::error::WorldError;
use crate::nbt::disk_nbt;

/// Location of one chunk: world coordinates in chunk units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ChunkPos {
    /// Chunk x.
    pub x: i32,
    /// Chunk z.
    pub z: i32,
}

/// One chunk exactly as stored: coordinates, sector type and the decoded
/// root compound of its disk NBT.
#[derive(Debug, Clone)]
pub struct StoredChunk {
    /// Chunk x.
    pub x: i32,
    /// Chunk z.
    pub z: i32,
    /// Sector compression byte (1 = gzip, 2 = zlib, 3 = raw).
    pub compression: u8,
    /// Root compound as found on disk; nothing is interpreted yet.
    pub nbt: NbtTag,
}

/// Region coordinates from a file name like `r.-1.0.mca`.
pub fn region_coords(path: &Path) -> Result<(i32, i32), WorldError> {
    let corrupt = |detail: String| WorldError::Corrupt {
        path: path.display().to_string(),
        detail,
    };
    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| corrupt("file name is not valid UTF-8".into()))?;
    let parts: Vec<&str> = name
        .strip_prefix("r.")
        .ok_or_else(|| corrupt(format!("{name} does not start with r.")))?
        .split('.')
        .collect();
    if parts.len() != 2 {
        return Err(corrupt(format!("{name} is not r.<x>.<z>")));
    }
    let x = parts[0]
        .parse()
        .map_err(|_| corrupt(format!("region x in {name} is not an integer")))?;
    let z = parts[1]
        .parse()
        .map_err(|_| corrupt(format!("region z in {name} is not an integer")))?;
    Ok((x, z))
}

/// Reads every present chunk of one `.mca` file through the 4 KiB header.
pub fn load_region(path: &Path) -> Result<Vec<StoredChunk>, WorldError> {
    let label = path.display().to_string();
    let corrupt = |detail: String| WorldError::Corrupt {
        path: label.clone(),
        detail,
    };
    let file = std::fs::read(path).map_err(|source| WorldError::Io {
        path: label.clone(),
        source,
    })?;
    if file.len() < 8192 {
        return Err(corrupt(format!(
            "{} bytes, shorter than the two 4 KiB headers",
            file.len()
        )));
    }
    let (region_x, region_z) = region_coords(path)?;
    let mut chunks = Vec::new();
    for index in 0..1024usize {
        if let Some(chunk) = read_slot(&file, index, region_x, region_z, &label, &mut |raw| {
            disk_nbt(&label, raw)
        })? {
            chunks.push(chunk);
        }
    }
    Ok(chunks)
}

/// Reads one header slot and decodes its payload, if the slot is present.
///
/// `decode` is injected so the data reader stays the only NBT knowledge.
fn read_slot(
    file: &[u8],
    index: usize,
    region_x: i32,
    region_z: i32,
    label: &str,
    decode: &mut dyn FnMut(&[u8]) -> Result<NbtTag, WorldError>,
) -> Result<Option<StoredChunk>, WorldError> {
    let corrupt = |detail: String| WorldError::Corrupt {
        path: label.to_owned(),
        detail,
    };
    let entry = &file[index * 4..index * 4 + 4];
    let sector_count = entry[3];
    if sector_count == 0 {
        return Ok(None); // empty slot, by layout
    }
    let header =
        (usize::from(entry[0]) << 16) | (usize::from(entry[1]) << 8) | usize::from(entry[2]);
    let offset = header * 4096;
    if offset + 5 > file.len() {
        return Err(corrupt(format!("slot {index} starts beyond the file")));
    }
    let length = u32::from_be_bytes([
        file[offset],
        file[offset + 1],
        file[offset + 2],
        file[offset + 3],
    ]) as usize;
    if length < 1 {
        return Err(corrupt(format!("slot {index} announces no payload")));
    }
    let payload_len = length - 1; // length counts the kind byte
    if offset + 5 + payload_len > file.len() {
        return Err(corrupt(format!(
            "{length} bytes at slot {index} run beyond the file"
        )));
    }
    if payload_len > usize::from(sector_count) * 4096 {
        return Err(corrupt(format!(
            "slot {index} payload of {payload_len} bytes exceeds its {sector_count} sectors"
        )));
    }
    let kind = file[offset + 4];
    let raw = decompress(label, kind, &file[offset + 5..offset + 5 + payload_len])?;
    let nbt = decode(&raw)?;
    Ok(Some(StoredChunk {
        x: region_x * 32 + (index % 32) as i32,
        z: region_z * 32 + (index / 32) as i32,
        compression: kind,
        nbt,
    }))
}

/// Decompresses one sector by its Anvil compression byte.
fn decompress(label: &str, kind: u8, data: &[u8]) -> Result<Vec<u8>, WorldError> {
    use std::io::Read;
    let mut out = Vec::new();
    let result = match kind {
        1 => flate2::read::GzDecoder::new(data).read_to_end(&mut out),
        2 => flate2::read::ZlibDecoder::new(data).read_to_end(&mut out),
        3 => {
            out.extend_from_slice(data);
            Ok(out.len())
        }
        _ => {
            return Err(WorldError::UnknownCompression {
                path: label.to_owned(),
                kind,
            })
        }
    };
    result.map_err(|source| WorldError::Io {
        path: label.to_owned(),
        source,
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two empty headers are a valid, empty region: every slot's sector
    /// count is zero, so nothing is read and nothing is guessed.
    #[test]
    fn reads_an_empty_region() {
        let dir = std::env::temp_dir().join("mine_rust-world-region-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("r.0.0.mca");
        let mut bytes = vec![0_u8; 8192];
        // One entry with a nonzero sector count pointing past the file must
        // fail loudly rather than slice out of bounds.
        bytes[0] = 244; // arbitrary high offset
        bytes[3] = 1; // one sector
        std::fs::write(&path, &bytes).expect("write empty region");
        let error = load_region(&path).expect_err("broken slot must fail");
        assert!(error.to_string().contains("beyond"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn reads_region_coordinates_from_the_file_name() {
        let path = Path::new("r.-1.3.mca");
        assert_eq!(region_coords(path).expect("coords"), (-1, 3));
    }

    #[test]
    fn rejects_an_unparsable_region_name() {
        let error = region_coords(Path::new("r.0.mca")).expect_err("one number");
        assert!(error.to_string().contains("is not r.<x>.<z>"));
    }
}
