//! NBT (Named Binary Tag) values and the network codec.
//!
//! Since protocol 764 (1.20.2) NBT sent over the network omits the name of the
//! root tag: a value is `Type (1 byte) + Payload`, while NBT files on disk
//! also carry a length-prefixed name for the root. Everything else follows
//! the public specification (<https://minecraft.wiki/w/NBT_format>):
//! big-endian numbers, names and strings as length-prefixed modified UTF-8
//! (the encoding of Java `DataOutput.writeUTF`), and homogeneous lists.

mod decode;
mod from_json;

use std::collections::BTreeMap;

/// Maximum nesting depth of compound and list tags, matching the vanilla
/// accounting limits for NBT received over the network.
pub const MAX_NBT_DEPTH: usize = 512;

/// Type id of TAG_End: terminates a compound and types an empty list.
const TAG_END: u8 = 0;

/// An NBT value.
///
/// Compounds keep their entries in a [`BTreeMap`]: NBT does not guarantee
/// entry order, and a sorted map makes encodings deterministic.
#[derive(Debug, Clone, PartialEq)]
pub enum NbtTag {
    /// Signed 8-bit integer (also used for booleans: 0 = false, 1 = true).
    Byte(i8),
    /// Signed 16-bit integer.
    Short(i16),
    /// Signed 32-bit integer.
    Int(i32),
    /// Signed 64-bit integer.
    Long(i64),
    /// IEEE 754 single-precision float.
    Float(f32),
    /// IEEE 754 double-precision float.
    Double(f64),
    /// Array of bytes; the length prefix counts the elements.
    ByteArray(Vec<i8>),
    /// Length-prefixed modified UTF-8 string.
    String(String),
    /// Homogeneous list of tags; an empty list is typed TAG_End on the wire.
    List(Vec<NbtTag>),
    /// Named tags terminated by TAG_End.
    Compound(BTreeMap<String, NbtTag>),
    /// Array of 32-bit integers.
    IntArray(Vec<i32>),
    /// Array of 64-bit integers.
    LongArray(Vec<i64>),
}

/// Errors produced while encoding, decoding or converting NBT.
#[derive(Debug, thiserror::Error)]
pub enum NbtError {
    /// The input ended before a complete value could be read.
    #[error("unexpected end of NBT input")]
    UnexpectedEof,
    /// A type byte was not a defined NBT type (TAG_End is only valid as a
    /// compound terminator or as the element type of an empty list).
    #[error("invalid NBT tag type {0}")]
    InvalidTagType(u8),
    /// A value nested deeper than [`MAX_NBT_DEPTH`] containers.
    #[error("NBT nesting exceeds the maximum depth")]
    DepthExceeded,
    /// A string or name did not fit its 16-bit length prefix.
    #[error("NBT string of {length} bytes exceeds the 65535 byte limit")]
    StringTooLong {
        /// Encoded length in bytes.
        length: usize,
    },
    /// A string was not valid modified UTF-8 or contained lone surrogates.
    #[error("invalid modified UTF-8 in NBT string")]
    InvalidNbtString,
    /// An array length prefix was negative.
    #[error("negative NBT array length {0}")]
    NegativeLength(i32),
    /// A length prefix promised more elements than the input holds.
    #[error("NBT length {length} exceeds the remaining input")]
    LengthExceedsInput {
        /// Promised number of elements.
        length: usize,
    },
    /// A list was built from elements of different types.
    #[error("NBT list contains mixed element types")]
    MixedList,
    /// A collection has more elements than an i32 length prefix can hold.
    #[error("NBT collection of {length} elements exceeds the i32 length prefix")]
    CollectionTooLong {
        /// Actual number of elements.
        length: usize,
    },
    /// A JSON value has no NBT representation.
    #[error("JSON null cannot be converted to NBT")]
    JsonNull,
    /// A JSON array mixed elements that convert to different NBT types.
    #[error("JSON array elements convert to different NBT types")]
    MixedJsonArray,
    /// A JSON number had no NBT representation.
    #[error("JSON number cannot be represented in NBT")]
    JsonNumber,
}

impl NbtTag {
    /// Tag type id used on the wire (see the public NBT specification).
    pub fn type_id(&self) -> u8 {
        match self {
            Self::Byte(_) => 1,
            Self::Short(_) => 2,
            Self::Int(_) => 3,
            Self::Long(_) => 4,
            Self::Float(_) => 5,
            Self::Double(_) => 6,
            Self::ByteArray(_) => 7,
            Self::String(_) => 8,
            Self::List(_) => 9,
            Self::Compound(_) => 10,
            Self::IntArray(_) => 11,
            Self::LongArray(_) => 12,
        }
    }

    /// Encodes this value in the network form: a type byte followed by the
    /// payload, without the root name used by NBT files on disk.
    pub fn encode_network(&self, out: &mut Vec<u8>) -> Result<(), NbtError> {
        out.push(self.type_id());
        self.encode_payload(out)
    }

    /// Convenience wrapper around [`Self::encode_network`].
    pub fn to_network_bytes(&self) -> Result<Vec<u8>, NbtError> {
        let mut out = Vec::new();
        self.encode_network(&mut out)?;
        Ok(out)
    }

    /// Appends the payload of this value (type byte excluded) to `out`.
    fn encode_payload(&self, out: &mut Vec<u8>) -> Result<(), NbtError> {
        match self {
            Self::Byte(value) => out.push(*value as u8),
            Self::Short(value) => out.extend_from_slice(&value.to_be_bytes()),
            Self::Int(value) => out.extend_from_slice(&value.to_be_bytes()),
            Self::Long(value) => out.extend_from_slice(&value.to_be_bytes()),
            Self::Float(value) => out.extend_from_slice(&value.to_bits().to_be_bytes()),
            Self::Double(value) => out.extend_from_slice(&value.to_bits().to_be_bytes()),
            Self::ByteArray(values) => {
                write_array_len(values.len(), out)?;
                out.extend(values.iter().map(|&value| value as u8));
            }
            Self::String(value) => write_mutf8(value, out)?,
            Self::List(items) => {
                let element_type = items.first().map_or(TAG_END, Self::type_id);
                if items.iter().any(|item| item.type_id() != element_type) {
                    return Err(NbtError::MixedList);
                }
                out.push(element_type);
                write_array_len(items.len(), out)?;
                for item in items {
                    item.encode_payload(out)?;
                }
            }
            Self::Compound(entries) => {
                for (name, tag) in entries {
                    out.push(tag.type_id());
                    write_mutf8(name, out)?;
                    tag.encode_payload(out)?;
                }
                out.push(TAG_END);
            }
            Self::IntArray(values) => {
                write_array_len(values.len(), out)?;
                for value in values {
                    out.extend_from_slice(&value.to_be_bytes());
                }
            }
            Self::LongArray(values) => {
                write_array_len(values.len(), out)?;
                for value in values {
                    out.extend_from_slice(&value.to_be_bytes());
                }
            }
        }
        Ok(())
    }
}

/// Writes a 32-bit element-count prefix.
fn write_array_len(len: usize, out: &mut Vec<u8>) -> Result<(), NbtError> {
    let len = i32::try_from(len).map_err(|_| NbtError::CollectionTooLong { length: len })?;
    out.extend_from_slice(&len.to_be_bytes());
    Ok(())
}

/// Writes a length-prefixed modified UTF-8 string (Java `writeUTF`): the
/// prefix counts the encoded bytes, U+0000 becomes `C0 80` and supplementary
/// characters become their surrogate pair as two 3-byte sequences.
fn write_mutf8(value: &str, out: &mut Vec<u8>) -> Result<(), NbtError> {
    let mut encoded = Vec::new();
    for unit in value.encode_utf16() {
        encode_unit(unit, &mut encoded);
    }
    let length = u16::try_from(encoded.len()).map_err(|_| NbtError::StringTooLong {
        length: encoded.len(),
    })?;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&encoded);
    Ok(())
}

/// Encodes one UTF-16 code unit as modified UTF-8.
fn encode_unit(unit: u16, out: &mut Vec<u8>) {
    if unit == 0 {
        out.push(0xC0);
        out.push(0x80);
    } else if unit < 0x80 {
        out.push(unit as u8);
    } else if unit < 0x800 {
        out.push(0xC0 | (unit >> 6) as u8);
        out.push(0x80 | (unit & 0x3F) as u8);
    } else {
        out.push(0xE0 | (unit >> 12) as u8);
        out.push(0x80 | ((unit >> 6) & 0x3F) as u8);
        out.push(0x80 | (unit & 0x3F) as u8);
    }
}

/// Decodes a modified UTF-8 byte sequence into a string (Java `readUTF`).
///
/// Four-byte UTF-8 sequences are rejected: supplementary characters arrive
/// as their CESU-8 surrogate pair instead.
pub(crate) fn decode_mutf8(bytes: &[u8]) -> Result<String, NbtError> {
    let mut units = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte < 0x80 {
            units.push(u16::from(byte));
            index += 1;
        } else if byte & 0xE0 == 0xC0 {
            let second = continuation(bytes, index)?;
            units.push(u16::from(byte & 0x1F) << 6 | u16::from(second & 0x3F));
            index += 2;
        } else if byte & 0xF0 == 0xE0 {
            let second = continuation(bytes, index)?;
            let third = continuation(bytes, index + 1)?;
            units.push(
                u16::from(byte & 0x0F) << 12
                    | u16::from(second & 0x3F) << 6
                    | u16::from(third & 0x3F),
            );
            index += 3;
        } else {
            return Err(NbtError::InvalidNbtString);
        }
    }
    String::from_utf16(&units).map_err(|_| NbtError::InvalidNbtString)
}

/// Reads the continuation byte that must follow position `index`.
fn continuation(bytes: &[u8], index: usize) -> Result<u8, NbtError> {
    let byte = bytes
        .get(index + 1)
        .copied()
        .ok_or(NbtError::InvalidNbtString)?;
    if byte & 0xC0 != 0x80 {
        return Err(NbtError::InvalidNbtString);
    }
    Ok(byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compound_of(entries: Vec<(&str, NbtTag)>) -> NbtTag {
        NbtTag::Compound(
            entries
                .into_iter()
                .map(|(name, tag)| (name.to_owned(), tag))
                .collect(),
        )
    }

    #[test]
    fn type_ids_match_the_specification() {
        let sample = compound_of(vec![("k", NbtTag::Byte(0))]);
        assert_eq!(NbtTag::Byte(0).type_id(), 1);
        assert_eq!(NbtTag::Short(0).type_id(), 2);
        assert_eq!(NbtTag::Int(0).type_id(), 3);
        assert_eq!(NbtTag::Long(0).type_id(), 4);
        assert_eq!(NbtTag::Float(0.0).type_id(), 5);
        assert_eq!(NbtTag::Double(0.0).type_id(), 6);
        assert_eq!(NbtTag::ByteArray(Vec::new()).type_id(), 7);
        assert_eq!(NbtTag::String(String::new()).type_id(), 8);
        assert_eq!(NbtTag::List(Vec::new()).type_id(), 9);
        assert_eq!(sample.type_id(), 10);
        assert_eq!(NbtTag::IntArray(Vec::new()).type_id(), 11);
        assert_eq!(NbtTag::LongArray(Vec::new()).type_id(), 12);
    }

    #[test]
    fn network_encoding_omits_the_root_name() {
        // The documented disk form of {"name": "Bananrama"} carries the root
        // name "hello world"; on the network only the type byte remains.
        let tag = compound_of(vec![("name", NbtTag::String("Bananrama".to_owned()))]);
        let bytes = tag.to_network_bytes().expect("encodable");
        assert_eq!(
            bytes,
            vec![
                0x0A, 0x08, 0x00, 0x04, b'n', b'a', b'm', b'e', 0x00, 0x09, b'B', b'a', b'n', b'a',
                b'n', b'r', b'a', b'm', b'a', 0x00,
            ]
        );
    }

    #[test]
    fn numeric_payloads_are_big_endian() {
        assert_eq!(
            NbtTag::Byte(-1).to_network_bytes().expect("encodable"),
            vec![0x01, 0xFF]
        );
        assert_eq!(
            NbtTag::Short(0x1234).to_network_bytes().expect("encodable"),
            vec![0x02, 0x12, 0x34]
        );
        assert_eq!(
            NbtTag::Int(1).to_network_bytes().expect("encodable"),
            vec![0x03, 0x00, 0x00, 0x00, 0x01]
        );
        assert_eq!(
            NbtTag::Long(-1).to_network_bytes().expect("encodable"),
            vec![0x04, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]
        );
        assert_eq!(
            NbtTag::Float(1.0).to_network_bytes().expect("encodable"),
            vec![0x05, 0x3F, 0x80, 0x00, 0x00]
        );
        assert_eq!(
            NbtTag::Double(1.0).to_network_bytes().expect("encodable"),
            vec![0x06, 0x3F, 0xF0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn strings_use_modified_utf8() {
        // Supplementary characters arrive as their CESU-8 surrogate pair;
        // U+0000 is escaped as C0 80. Both differ from plain UTF-8.
        assert_eq!(
            NbtTag::String("🌍".to_owned())
                .to_network_bytes()
                .expect("encodable"),
            vec![0x08, 0x00, 0x06, 0xED, 0xA0, 0xBC, 0xED, 0xBC, 0x8D]
        );
        assert_eq!(
            NbtTag::String("a\0b".to_owned())
                .to_network_bytes()
                .expect("encodable"),
            vec![0x08, 0x00, 0x04, b'a', 0xC0, 0x80, b'b']
        );
    }

    #[test]
    fn empty_list_uses_end_element_type() {
        assert_eq!(
            NbtTag::List(Vec::new())
                .to_network_bytes()
                .expect("encodable"),
            vec![0x09, 0x00, 0x00, 0x00, 0x00, 0x00]
        );
    }

    #[test]
    fn mixed_list_is_rejected() {
        let tag = NbtTag::List(vec![NbtTag::Byte(1), NbtTag::Short(2)]);
        let err = tag.to_network_bytes().expect_err("mixed");
        assert!(matches!(err, NbtError::MixedList));
    }

    #[test]
    fn oversized_string_is_rejected() {
        let tag = NbtTag::String("a".repeat(65536));
        let err = tag.to_network_bytes().expect_err("too long");
        assert!(matches!(err, NbtError::StringTooLong { length: 65536 }));
    }
}
