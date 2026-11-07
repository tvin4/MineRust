//! Decoding of the network NBT form (no root name).

use std::collections::BTreeMap;

use super::{decode_mutf8, NbtError, NbtTag, MAX_NBT_DEPTH, TAG_END};

impl NbtTag {
    /// Decodes a value from the network NBT form: a type byte followed by
    /// the payload, with no name for the root tag.
    ///
    /// Returns the value and the number of bytes consumed; whether trailing
    /// bytes are acceptable is up to the caller.
    pub fn decode_network(bytes: &[u8]) -> Result<(Self, usize), NbtError> {
        let mut cursor = Cursor {
            data: bytes,
            position: 0,
        };
        let tag = cursor.read_value(0)?;
        Ok((tag, cursor.position))
    }
}

/// Read cursor over a byte slice with the primitive reads NBT needs.
struct Cursor<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn remaining(&self) -> usize {
        self.data.len() - self.position
    }

    /// Advances past `len` bytes and returns them.
    fn take(&mut self, len: usize) -> Result<&'a [u8], NbtError> {
        let end = self
            .position
            .checked_add(len)
            .ok_or(NbtError::UnexpectedEof)?;
        let bytes = self
            .data
            .get(self.position..end)
            .ok_or(NbtError::UnexpectedEof)?;
        self.position = end;
        Ok(bytes)
    }

    fn read_u8(&mut self) -> Result<u8, NbtError> {
        Ok(self.take(1)?[0])
    }

    /// Reads an `N`-byte big-endian value's raw bytes.
    fn read_be<const N: usize>(&mut self) -> Result<[u8; N], NbtError> {
        let bytes = self.take(N)?;
        let mut array = [0u8; N];
        array.copy_from_slice(bytes);
        Ok(array)
    }

    /// Reads a 32-bit element-count prefix.
    fn read_len(&mut self) -> Result<usize, NbtError> {
        let len = i32::from_be_bytes(self.read_be()?);
        usize::try_from(len).map_err(|_| NbtError::NegativeLength(len))
    }

    /// Reads a 16-bit byte-length prefix and returns the referenced bytes.
    fn take_sized(&mut self) -> Result<&'a [u8], NbtError> {
        let len = u16::from_be_bytes(self.read_be()?) as usize;
        self.take(len)
    }

    /// Reads a type byte plus payload; `depth` counts enclosing containers.
    fn read_value(&mut self, depth: usize) -> Result<NbtTag, NbtError> {
        let type_id = self.read_u8()?;
        self.read_payload(type_id, depth)
    }

    fn read_payload(&mut self, type_id: u8, depth: usize) -> Result<NbtTag, NbtError> {
        match type_id {
            1 => Ok(NbtTag::Byte(self.read_be::<1>()?[0] as i8)),
            2 => Ok(NbtTag::Short(i16::from_be_bytes(self.read_be()?))),
            3 => Ok(NbtTag::Int(i32::from_be_bytes(self.read_be()?))),
            4 => Ok(NbtTag::Long(i64::from_be_bytes(self.read_be()?))),
            5 => Ok(NbtTag::Float(f32::from_bits(u32::from_be_bytes(
                self.read_be()?,
            )))),
            6 => Ok(NbtTag::Double(f64::from_bits(u64::from_be_bytes(
                self.read_be()?,
            )))),
            7 => self.read_byte_array(),
            8 => Ok(NbtTag::String(decode_mutf8(self.take_sized()?)?)),
            9 => self.read_list(depth),
            10 => self.read_compound(depth),
            11 => self.read_int_array(),
            12 => self.read_long_array(),
            other => Err(NbtError::InvalidTagType(other)),
        }
    }

    fn read_byte_array(&mut self) -> Result<NbtTag, NbtError> {
        let len = self.read_len()?;
        // Every element occupies at least one byte, so a length larger than
        // the remaining input cannot be satisfied and would over-allocate.
        if len > self.remaining() {
            return Err(NbtError::LengthExceedsInput { length: len });
        }
        let bytes = self.take(len)?;
        Ok(NbtTag::ByteArray(
            bytes.iter().map(|&byte| byte as i8).collect(),
        ))
    }

    fn read_int_array(&mut self) -> Result<NbtTag, NbtError> {
        let len = self.read_len()?;
        if len > self.remaining() / 4 {
            return Err(NbtError::LengthExceedsInput { length: len });
        }
        let bytes = self.take(len * 4)?;
        let (chunks, tail) = bytes.as_chunks::<4>();
        debug_assert!(tail.is_empty(), "length is a multiple of 4");
        let values = chunks
            .iter()
            .map(|&chunk| i32::from_be_bytes(chunk))
            .collect();
        Ok(NbtTag::IntArray(values))
    }

    fn read_long_array(&mut self) -> Result<NbtTag, NbtError> {
        let len = self.read_len()?;
        if len > self.remaining() / 8 {
            return Err(NbtError::LengthExceedsInput { length: len });
        }
        let bytes = self.take(len * 8)?;
        let (chunks, tail) = bytes.as_chunks::<8>();
        debug_assert!(tail.is_empty(), "length is a multiple of 8");
        let values = chunks
            .iter()
            .map(|&chunk| i64::from_be_bytes(chunk))
            .collect();
        Ok(NbtTag::LongArray(values))
    }

    fn read_list(&mut self, depth: usize) -> Result<NbtTag, NbtError> {
        let element_type = self.read_u8()?;
        let count = i32::from_be_bytes(self.read_be()?);
        if count <= 0 {
            // Vanilla writes empty lists with TAG_End and accepts any element
            // type when the length is not positive.
            return Ok(NbtTag::List(Vec::new()));
        }
        if element_type == TAG_END {
            return Err(NbtError::InvalidTagType(TAG_END));
        }
        let count = count as usize;
        if count > self.remaining() {
            return Err(NbtError::LengthExceedsInput { length: count });
        }
        let inner_depth = depth + 1;
        if inner_depth > MAX_NBT_DEPTH {
            return Err(NbtError::DepthExceeded);
        }
        let mut items = Vec::with_capacity(count);
        for _ in 0..count {
            items.push(self.read_payload(element_type, inner_depth)?);
        }
        Ok(NbtTag::List(items))
    }

    fn read_compound(&mut self, depth: usize) -> Result<NbtTag, NbtError> {
        let inner_depth = depth + 1;
        if inner_depth > MAX_NBT_DEPTH {
            return Err(NbtError::DepthExceeded);
        }
        let mut entries = BTreeMap::new();
        loop {
            let type_id = self.read_u8()?;
            if type_id == TAG_END {
                break;
            }
            let name = decode_mutf8(self.take_sized()?)?;
            let value = self.read_payload(type_id, inner_depth)?;
            entries.insert(name, value);
        }
        Ok(NbtTag::Compound(entries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a compound nested `depth` containers deep.
    fn nested_compound(depth: usize) -> NbtTag {
        let mut tag = NbtTag::Compound(BTreeMap::new());
        for _ in 1..depth {
            tag = NbtTag::Compound(BTreeMap::from([("n".to_owned(), tag)]));
        }
        tag
    }

    /// A compound exercising every tag type, including nested containers.
    fn sample_tag() -> NbtTag {
        NbtTag::Compound(BTreeMap::from([
            ("byte".to_owned(), NbtTag::Byte(-1)),
            ("short".to_owned(), NbtTag::Short(-32768)),
            ("int".to_owned(), NbtTag::Int(i32::MIN)),
            ("long".to_owned(), NbtTag::Long(i64::MIN)),
            ("float".to_owned(), NbtTag::Float(0.5)),
            ("double".to_owned(), NbtTag::Double(0.1)),
            ("bytes".to_owned(), NbtTag::ByteArray(vec![1, -1, 0])),
            ("string".to_owned(), NbtTag::String("🌍 мир\0".to_owned())),
            (
                "list".to_owned(),
                NbtTag::List(vec![NbtTag::Int(1), NbtTag::Int(2)]),
            ),
            ("empty".to_owned(), NbtTag::List(Vec::new())),
            (
                "compound".to_owned(),
                NbtTag::Compound(BTreeMap::from([("nested".to_owned(), NbtTag::Byte(1))])),
            ),
            ("ints".to_owned(), NbtTag::IntArray(vec![1, -1])),
            ("longs".to_owned(), NbtTag::LongArray(vec![1, -1])),
        ]))
    }

    #[test]
    fn network_root_compound_omits_the_root_name() {
        // The documented disk encoding of {"name": "Bananrama"} starts with
        // the root name; the network form keeps everything after the type id.
        let bytes = vec![
            0x0A, 0x08, 0x00, 0x04, b'n', b'a', b'm', b'e', 0x00, 0x09, b'B', b'a', b'n', b'a',
            b'n', b'r', b'a', b'm', b'a', 0x00,
        ];
        let (tag, consumed) = NbtTag::decode_network(&bytes).expect("decodable");
        assert_eq!(consumed, bytes.len());
        assert_eq!(
            tag,
            NbtTag::Compound(BTreeMap::from([(
                "name".to_owned(),
                NbtTag::String("Bananrama".to_owned())
            )]))
        );
    }

    #[test]
    fn every_tag_type_roundtrips() {
        let tag = sample_tag();
        let bytes = tag.to_network_bytes().expect("encodable");
        let (decoded, consumed) = NbtTag::decode_network(&bytes).expect("decodable");
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded, tag);
    }

    #[test]
    fn decode_reports_consumed_bytes() {
        let mut bytes = sample_tag().to_network_bytes().expect("encodable");
        bytes.push(0xAA);
        let (decoded, consumed) = NbtTag::decode_network(&bytes).expect("decodable");
        assert_eq!(consumed, bytes.len() - 1);
        assert_eq!(decoded, sample_tag());
    }

    #[test]
    fn unknown_tag_types_are_rejected() {
        let err = NbtTag::decode_network(&[0x0D]).expect_err("unknown type");
        assert!(matches!(err, NbtError::InvalidTagType(13)));

        // TAG_End is only a compound terminator, never a standalone value.
        let err = NbtTag::decode_network(&[0x00]).expect_err("end tag");
        assert!(matches!(err, NbtError::InvalidTagType(0)));
    }

    #[test]
    fn truncated_values_are_rejected() {
        // Int payload cut short.
        let err = NbtTag::decode_network(&[0x03, 0x00]).expect_err("truncated int");
        assert!(matches!(err, NbtError::UnexpectedEof));

        // Compound starts a String entry but the name never arrives.
        let err = NbtTag::decode_network(&[0x0A, 0x08]).expect_err("cut compound");
        assert!(matches!(err, NbtError::UnexpectedEof));

        // String length promises more bytes than remain.
        let err = NbtTag::decode_network(&[0x08, 0x00, 0x05, b'a']).expect_err("short string");
        assert!(matches!(err, NbtError::UnexpectedEof));
    }

    #[test]
    fn negative_array_lengths_are_rejected() {
        let err = NbtTag::decode_network(&[0x07, 0xFF, 0xFF, 0xFF, 0xFF])
            .expect_err("negative byte array");
        assert!(matches!(err, NbtError::NegativeLength(-1)));

        let err = NbtTag::decode_network(&[0x0B, 0xFF, 0xFF, 0xFF, 0xFF])
            .expect_err("negative int array");
        assert!(matches!(err, NbtError::NegativeLength(-1)));
    }

    #[test]
    fn lengths_beyond_the_input_are_rejected() {
        // A list of five bytes with no payload behind it.
        let err = NbtTag::decode_network(&[0x09, 0x01, 0x00, 0x00, 0x00, 0x05])
            .expect_err("missing list payload");
        assert!(matches!(err, NbtError::LengthExceedsInput { length: 5 }));

        let err = NbtTag::decode_network(&[0x07, 0x00, 0x00, 0x00, 0x05])
            .expect_err("missing array payload");
        assert!(matches!(err, NbtError::LengthExceedsInput { length: 5 }));
    }

    #[test]
    fn empty_lists_accept_any_element_type() {
        // A negative count with element type TAG_End: vanilla parsers accept
        // it as an empty list.
        let bytes = [0x09, 0x01, 0xFF, 0xFF, 0xFF, 0xFF];
        let (tag, consumed) = NbtTag::decode_network(&bytes).expect("decodable");
        assert_eq!(consumed, bytes.len());
        assert_eq!(tag, NbtTag::List(Vec::new()));

        // A positive count with element type TAG_End is invalid.
        let err = NbtTag::decode_network(&[0x09, 0x00, 0x00, 0x00, 0x00, 0x01])
            .expect_err("end-typed non-empty list");
        assert!(matches!(err, NbtError::InvalidTagType(0)));
    }

    #[test]
    fn depth_limit_allows_512_and_rejects_513() {
        let bytes = nested_compound(512).to_network_bytes().expect("encodable");
        NbtTag::decode_network(&bytes).expect("512 levels decode");

        let bytes = nested_compound(513).to_network_bytes().expect("encodable");
        let err = NbtTag::decode_network(&bytes).expect_err("513 levels");
        assert!(matches!(err, NbtError::DepthExceeded));
    }

    #[test]
    fn strings_roundtrip_through_modified_utf8() {
        for text in ["", "plain", "Привет", "🌍 мир", "a\0b"] {
            let tag = NbtTag::String(text.to_owned());
            let bytes = tag.to_network_bytes().expect("encodable");
            let (decoded, consumed) = NbtTag::decode_network(&bytes).expect("decodable");
            assert_eq!(consumed, bytes.len());
            assert_eq!(decoded, tag, "roundtrip of {text:?}");
        }
    }
}
