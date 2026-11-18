//! The `.mrchunk` document: one chunk's cells that differ from the template
//! (Шаг 22b).
//!
//! Layout (little-endian, `VarInt` = the protocol's unsigned LEB128):
//!
//! ```text
//! magic     4 bytes  "MRSV"
//! version   u8       1
//! chunk_x   i32
//! chunk_z   i32
//! count     u32      number of changed cells
//! count ×   u32      flat cell index (section slot then the wire cell)
//!           VarInt   state id
//! crc       u32      CRC-32 (IEEE) of every byte before it
//! ```
//!
//! The cell index is section-major, not a world coordinate, so the file does
//! not repeat the chunk origin; `u32` rather than the plan's `u16` because
//! 24 sections × 4096 cells = 98304 cells do not fit 16 bits (deviation
//! recorded in `docs/WORLD-SAVE.md`).

use thiserror::Error;

/// File magic of a chunk diff.
pub const MAGIC: [u8; 4] = *b"MRSV";

/// Current format version.
pub const VERSION: u8 = 1;

/// Block cells one chunk payload holds: 24 sections of 4096.
pub const CELLS_PER_CHUNK: u32 = 24 * 4096;

/// Bytes before the first change record.
const HEADER: usize = 4 + 1 + 4 + 4 + 4;

/// Bytes of the trailing checksum.
const CHECKSUM: usize = 4;

/// One chunk's saved cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkDiff {
    /// Chunk x coordinate.
    pub chunk_x: i32,
    /// Chunk z coordinate.
    pub chunk_z: i32,
    /// Changed cells, ascending: `(cell index, state id)`.
    pub changes: Vec<(u32, i32)>,
}

impl ChunkDiff {
    /// Whether every cell is back at the template value: the writer deletes
    /// the file instead of storing an empty document.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Why a document cannot be read.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FormatError {
    /// The file ends inside its fixed header or its checksum.
    #[error("the document is {len} bytes, shorter than its {need}-byte frame")]
    TooShort {
        /// Actual length.
        len: usize,
        /// Smallest valid length.
        need: usize,
    },
    /// The first four bytes are not [`MAGIC`].
    #[error("the document does not start with the MRSV magic")]
    Magic,
    /// The version byte is not [`VERSION`].
    #[error("format version {0} is not supported (expected {VERSION})")]
    Version(u8),
    /// The stored CRC-32 does not match the body.
    #[error("the document checksum does not match its contents")]
    Checksum,
    /// A cell index is outside [`CELLS_PER_CHUNK`].
    #[error("cell index {index} is outside the chunk")]
    Cell {
        /// The offending index.
        index: u32,
    },
    /// Two records are not in ascending cell order.
    #[error("cell records are not in ascending order at {index}")]
    Order {
        /// The cell that repeats or goes back.
        index: u32,
    },
    /// The document ends before the declared number of changes.
    #[error("the document ends before its declared {declared} changes")]
    Truncated {
        /// Number of changes the header declares.
        declared: u32,
    },
    /// Bytes remain after the last declared change.
    #[error("the document carries {extra} trailing bytes")]
    Trailing {
        /// How many bytes are left over.
        extra: usize,
    },
    /// A state id does not fit the signed 32-bit range.
    #[error("a state id does not fit the format")]
    State,
    /// The document names another chunk than its file name.
    #[error(
        "the document names chunk ({found_x}, {found_z}) but the file names ({expected_x}, {expected_z})"
    )]
    Coordinate {
        /// Chunk x stored in the document.
        found_x: i32,
        /// Chunk z stored in the document.
        found_z: i32,
        /// Chunk x of the file name.
        expected_x: i32,
        /// Chunk z of the file name.
        expected_z: i32,
    },
}

/// Encodes one chunk diff, checksum included.
pub fn encode(diff: &ChunkDiff) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + diff.changes.len() * 6 + CHECKSUM);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&diff.chunk_x.to_le_bytes());
    out.extend_from_slice(&diff.chunk_z.to_le_bytes());
    out.extend_from_slice(&(diff.changes.len() as u32).to_le_bytes());
    for &(cell, state) in &diff.changes {
        out.extend_from_slice(&cell.to_le_bytes());
        write_varint(&mut out, state as u32);
    }
    let crc = crc32(&out);
    out.extend_from_slice(&crc.to_le_bytes());
    out
}

/// Decodes one chunk diff, verifying the checksum and every index.
pub fn decode(bytes: &[u8]) -> Result<ChunkDiff, FormatError> {
    if bytes.len() < HEADER + CHECKSUM {
        return Err(FormatError::TooShort {
            len: bytes.len(),
            need: HEADER + CHECKSUM,
        });
    }
    let (body, tail) = bytes.split_at(bytes.len() - CHECKSUM);
    let stored = u32::from_le_bytes([tail[0], tail[1], tail[2], tail[3]]);
    if crc32(body) != stored {
        return Err(FormatError::Checksum);
    }
    if body[0..4] != MAGIC {
        return Err(FormatError::Magic);
    }
    let version = body[4];
    if version != VERSION {
        return Err(FormatError::Version(version));
    }
    let chunk_x = i32::from_le_bytes([body[5], body[6], body[7], body[8]]);
    let chunk_z = i32::from_le_bytes([body[9], body[10], body[11], body[12]]);
    let declared = u32::from_le_bytes([body[13], body[14], body[15], body[16]]);
    let mut at = HEADER;
    let mut changes = Vec::new();
    let mut last = None;
    for _ in 0..declared {
        let Some(cell_bytes) = body.get(at..at + 4) else {
            return Err(FormatError::Truncated { declared });
        };
        let cell = u32::from_le_bytes([cell_bytes[0], cell_bytes[1], cell_bytes[2], cell_bytes[3]]);
        at += 4;
        let (state, next) = read_varint(body, at).ok_or(FormatError::Truncated { declared })?;
        at = next;
        if cell >= CELLS_PER_CHUNK {
            return Err(FormatError::Cell { index: cell });
        }
        if last.is_some_and(|previous| cell <= previous) {
            return Err(FormatError::Order { index: cell });
        }
        last = Some(cell);
        let state = i32::try_from(state).map_err(|_| FormatError::State)?;
        changes.push((cell, state));
    }
    if at != body.len() {
        return Err(FormatError::Trailing {
            extra: body.len() - at,
        });
    }
    Ok(ChunkDiff {
        chunk_x,
        chunk_z,
        changes,
    })
}

/// Appends `value` as an unsigned LEB128 VarInt.
fn write_varint(out: &mut Vec<u8>, value: u32) {
    let mut value = value;
    loop {
        let byte = (value & 0x7F) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Reads an unsigned LEB128 VarInt at `at`; `None` on a short or overlong one.
fn read_varint(bytes: &[u8], mut at: usize) -> Option<(u32, usize)> {
    let mut value = 0_u32;
    let mut shift = 0;
    loop {
        let byte = *bytes.get(at)?;
        at += 1;
        if shift == 28 && byte > 0x0F {
            return None;
        }
        value |= u32::from(byte & 0x7F) << shift;
        if byte & 0x80 == 0 {
            return Some((value, at));
        }
        shift += 7;
        if shift > 28 {
            return None;
        }
    }
}

/// CRC-32 (IEEE 802.3, the `crc32fast`/zlib polynomial), bitwise.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The standard CRC-32 check value; a wrong polynomial or reflection
    /// cannot pass it.
    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    /// A diff round-trips, including negative chunk coordinates and the
    /// highest cell of the chunk.
    #[test]
    fn a_diff_roundtrips() {
        let diff = ChunkDiff {
            chunk_x: -1,
            chunk_z: -2,
            changes: vec![(0, 0), (32872, 18445), (CELLS_PER_CHUNK - 1, 1)],
        };
        assert_eq!(decode(&encode(&diff)).expect("decodable"), diff);
    }

    /// An empty diff (a chunk back at the template) is a valid document.
    #[test]
    fn an_empty_diff_roundtrips() {
        let diff = ChunkDiff {
            chunk_x: 3,
            chunk_z: -4,
            changes: Vec::new(),
        };
        let bytes = encode(&diff);
        assert_eq!(bytes.len(), HEADER + CHECKSUM);
        assert_eq!(decode(&bytes).expect("decodable"), diff);
        assert!(diff.is_empty());
    }

    /// A flipped byte is caught by the checksum, not applied.
    #[test]
    fn a_damaged_document_is_refused() {
        let diff = ChunkDiff {
            chunk_x: 0,
            chunk_z: 0,
            changes: vec![(7, 1)],
        };
        let mut bytes = encode(&diff);
        bytes[HEADER] ^= 0x01;
        assert_eq!(decode(&bytes), Err(FormatError::Checksum));
    }

    /// Another magic and another version are named, not guessed.
    #[test]
    fn a_foreign_magic_or_version_is_named() {
        let diff = ChunkDiff {
            chunk_x: 0,
            chunk_z: 0,
            changes: Vec::new(),
        };
        let mut bytes = encode(&diff);
        bytes[0] = b'X';
        let crc = crc32(&bytes[..bytes.len() - CHECKSUM]);
        let len = bytes.len();
        bytes[len - CHECKSUM..].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(decode(&bytes), Err(FormatError::Magic));

        let mut bytes = encode(&diff);
        bytes[4] = 9;
        let crc = crc32(&bytes[..bytes.len() - CHECKSUM]);
        let len = bytes.len();
        bytes[len - CHECKSUM..].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(decode(&bytes), Err(FormatError::Version(9)));
    }

    /// Records must ascend, and a cell outside the chunk is refused.
    #[test]
    fn a_bad_cell_record_is_refused() {
        let diff = ChunkDiff {
            chunk_x: 0,
            chunk_z: 0,
            changes: vec![(8, 1), (7, 2)],
        };
        assert_eq!(decode(&encode(&diff)), Err(FormatError::Order { index: 7 }));

        let diff = ChunkDiff {
            chunk_x: 0,
            chunk_z: 0,
            changes: vec![(CELLS_PER_CHUNK, 1)],
        };
        assert_eq!(
            decode(&encode(&diff)),
            Err(FormatError::Cell {
                index: CELLS_PER_CHUNK
            })
        );
    }
}
