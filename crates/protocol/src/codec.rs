//! Primitive protocol codecs: VarInt, protocol strings and fixed-width integers.
//!
//! Decoding works over a byte slice ([`PacketReader`]); encoding appends to a
//! [`PacketWriter`]. Both directions are synchronous and do no I/O, so they can
//! be unit tested in isolation; the async network layer in the `net` crate
//! builds on top of them.

/// Maximum number of bytes a single VarInt may occupy.
// SOURCE: wiki, unverified: VarInt encodings are documented; a real client
// only proves them when it accepts our frames.
pub const MAX_VARINT_BYTES: usize = 5;

/// General protocol limit for protocol strings: maximum number of UTF-16 code
/// units (context-specific limits are usually smaller).
pub const MAX_STRING_CHARS: usize = 32767;

/// A 128-bit UUID as it appears on the wire: 16 bytes, big-endian.
pub type Uuid = [u8; 16];

/// A block position packed into one 64-bit word: x as a signed 26-bit
/// integer, then z (26 bits), then y (12 bits) — see the public protocol
/// documentation (<https://minecraft.wiki/wiki/Java_Edition_protocol>).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Position {
    /// Block X coordinate.
    pub x: i32,
    /// Block Y coordinate.
    pub y: i32,
    /// Block Z coordinate.
    pub z: i32,
}

/// Errors produced while encoding or decoding protocol data.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// The input ended before a complete value could be read.
    #[error("unexpected end of input")]
    UnexpectedEof,
    /// A VarInt continued past its fifth byte.
    #[error("VarInt is longer than 5 bytes")]
    VarIntTooLong,
    /// A length prefix was negative (the wire format has no such values).
    #[error("negative length prefix {0}")]
    NegativeLength(i32),
    /// A string exceeded its context-specific length limit, counted in UTF-16
    /// code units as required by the protocol.
    #[error("string of {length} UTF-16 code units exceeds limit {limit}")]
    StringTooLong {
        /// Length of the string in UTF-16 code units.
        length: usize,
        /// Maximum allowed length in UTF-16 code units.
        limit: usize,
    },
    /// A string was not valid UTF-8.
    #[error("string is not valid UTF-8: {0}")]
    InvalidUtf8(#[from] std::str::Utf8Error),
    /// A packet body exceeded the protocol packet size limit.
    #[error("packet of {length} bytes exceeds limit {limit}")]
    FrameTooLong {
        /// Actual packet length in bytes.
        length: usize,
        /// Maximum allowed packet length in bytes.
        limit: usize,
    },
    /// The packet length field was encoded in more than 3 bytes, which the
    /// protocol forbids even when the value itself fits the limit.
    #[error("packet length field is longer than 3 bytes")]
    FrameLengthEncodingTooLong,
    /// A packet id could not be represented as a non-negative `i32`.
    #[error("invalid packet id {0}")]
    InvalidPacketId(i32),
    /// A packet id did not match the one expected in the current state.
    #[error("unexpected packet id {id}, expected {expected}")]
    UnexpectedPacketId {
        /// Packet id that was read.
        id: u32,
        /// Packet id expected by the decoder.
        expected: u32,
    },
    /// Unread bytes remained after a packet payload was decoded.
    #[error("{remaining} trailing bytes after packet payload")]
    TrailingBytes {
        /// Number of unread bytes.
        remaining: usize,
    },
    /// A handshake requested an unknown next state.
    #[error("invalid handshake next state {0}")]
    InvalidNextState(i32),
    /// An NBT value could not be decoded.
    #[error("invalid NBT: {0}")]
    Nbt(#[from] crate::nbt::NbtError),
}

/// Encodes `value` into `out` using the Minecraft VarInt encoding.
///
/// VarInts are little-endian groups of 7 bits with a continuation flag in the
/// high bit; negative values always occupy the full 5 bytes.
pub fn encode_varint(value: i32, out: &mut Vec<u8>) {
    let mut value = value as u32;
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Decodes a VarInt from the front of `bytes`.
///
/// Returns the value and the number of bytes consumed. Unnecessarily long but
/// non-continuing encodings are accepted, as permitted by the protocol.
pub fn decode_varint(bytes: &[u8]) -> Result<(i32, usize), CodecError> {
    let mut value: u32 = 0;
    for (position, &byte) in bytes.iter().take(MAX_VARINT_BYTES).enumerate() {
        value |= u32::from(byte & 0x7F) << (position * 7);
        if byte & 0x80 == 0 {
            return Ok((value as i32, position + 1));
        }
    }
    if bytes.len() < MAX_VARINT_BYTES {
        Err(CodecError::UnexpectedEof)
    } else {
        Err(CodecError::VarIntTooLong)
    }
}

/// Cursor over the payload of a single packet.
#[derive(Debug)]
pub struct PacketReader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> PacketReader<'a> {
    /// Creates a reader over `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    /// Number of bytes that have not been read yet.
    pub fn remaining(&self) -> usize {
        self.data.len() - self.position
    }

    /// Returns `true` when every byte has been read.
    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// Fails with [`CodecError::TrailingBytes`] if unread bytes remain.
    pub fn ensure_empty(&self) -> Result<(), CodecError> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(CodecError::TrailingBytes {
                remaining: self.remaining(),
            })
        }
    }

    /// Reads a VarInt.
    pub fn read_varint(&mut self) -> Result<i32, CodecError> {
        let (value, consumed) = decode_varint(&self.data[self.position..])?;
        self.position += consumed;
        Ok(value)
    }

    /// Reads a protocol string limited to `max_chars` UTF-16 code units.
    pub fn read_string(&mut self, max_chars: usize) -> Result<String, CodecError> {
        let byte_len = self.read_varint()?;
        let byte_len =
            usize::try_from(byte_len).map_err(|_| CodecError::NegativeLength(byte_len))?;
        let end = self
            .position
            .checked_add(byte_len)
            .ok_or(CodecError::UnexpectedEof)?;
        let bytes = self
            .data
            .get(self.position..end)
            .ok_or(CodecError::UnexpectedEof)?;
        let text = std::str::from_utf8(bytes)?;
        let length = text.encode_utf16().count();
        if length > max_chars {
            return Err(CodecError::StringTooLong {
                length,
                limit: max_chars,
            });
        }
        self.position = end;
        Ok(text.to_owned())
    }

    /// Reads a single unsigned byte.
    pub fn read_u8(&mut self) -> Result<u8, CodecError> {
        Ok(self.read_array::<1>()?[0])
    }

    /// Reads a single signed byte.
    pub fn read_i8(&mut self) -> Result<i8, CodecError> {
        Ok(self.read_u8()? as i8)
    }

    /// Reads a big-endian `u16`.
    pub fn read_u16(&mut self) -> Result<u16, CodecError> {
        let bytes = self.read_array::<2>()?;
        Ok(u16::from_be_bytes(bytes))
    }

    /// Reads a big-endian `i32`.
    pub fn read_i32(&mut self) -> Result<i32, CodecError> {
        let bytes = self.read_array::<4>()?;
        Ok(i32::from_be_bytes(bytes))
    }

    /// Reads a big-endian `i64`.
    pub fn read_i64(&mut self) -> Result<i64, CodecError> {
        let bytes = self.read_array::<8>()?;
        Ok(i64::from_be_bytes(bytes))
    }

    /// Reads a big-endian 32-bit float.
    pub fn read_f32(&mut self) -> Result<f32, CodecError> {
        let bytes = self.read_array::<4>()?;
        Ok(f32::from_be_bytes(bytes))
    }

    /// Reads a big-endian 64-bit float.
    pub fn read_f64(&mut self) -> Result<f64, CodecError> {
        let bytes = self.read_array::<8>()?;
        Ok(f64::from_be_bytes(bytes))
    }

    /// Reads a packed block position.
    pub fn read_position(&mut self) -> Result<Position, CodecError> {
        let packed = self.read_i64()?;
        Ok(Position {
            x: (packed >> 38) as i32,
            y: ((packed << 52) >> 52) as i32,
            z: ((packed << 26) >> 38) as i32,
        })
    }

    /// Reads a boolean: one byte, non-zero means `true`.
    pub fn read_bool(&mut self) -> Result<bool, CodecError> {
        Ok(self.read_array::<1>()?[0] != 0)
    }

    /// Reads a 16-byte UUID.
    pub fn read_uuid(&mut self) -> Result<Uuid, CodecError> {
        self.read_array::<16>()
    }

    /// Reads a network NBT value (type byte plus payload, no root name) and
    /// advances past the bytes it consumed.
    pub fn read_nbt(&mut self) -> Result<crate::nbt::NbtTag, CodecError> {
        let (tag, consumed) = crate::nbt::NbtTag::decode_network(&self.data[self.position..])?;
        self.position += consumed;
        Ok(tag)
    }

    /// Reads exactly `N` bytes.
    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        let end = self
            .position
            .checked_add(N)
            .ok_or(CodecError::UnexpectedEof)?;
        let bytes = self
            .data
            .get(self.position..end)
            .ok_or(CodecError::UnexpectedEof)?;
        let mut array = [0u8; N];
        array.copy_from_slice(bytes);
        self.position = end;
        Ok(array)
    }
}

/// Builder that appends protocol values to a byte buffer.
#[derive(Debug, Default)]
pub struct PacketWriter {
    buffer: Vec<u8>,
}

impl PacketWriter {
    /// Creates an empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a VarInt.
    pub fn write_varint(&mut self, value: i32) {
        encode_varint(value, &mut self.buffer);
    }

    /// Appends a protocol string limited to `max_chars` UTF-16 code units.
    ///
    /// The byte-length limit implied by `max_chars` (3 bytes per code unit) is
    /// always satisfied when the code-unit limit holds, so only the latter is
    /// checked.
    pub fn write_string(&mut self, value: &str, max_chars: usize) -> Result<(), CodecError> {
        let length = value.encode_utf16().count();
        if length > max_chars {
            return Err(CodecError::StringTooLong {
                length,
                limit: max_chars,
            });
        }
        encode_varint(value.len() as i32, &mut self.buffer);
        self.buffer.extend_from_slice(value.as_bytes());
        Ok(())
    }

    /// Appends an unsigned byte.
    pub fn write_u8(&mut self, value: u8) {
        self.buffer.push(value);
    }

    /// Appends a signed byte.
    pub fn write_i8(&mut self, value: i8) {
        self.buffer.push(value as u8);
    }

    /// Appends a big-endian `u16`.
    pub fn write_u16(&mut self, value: u16) {
        self.buffer.extend_from_slice(&value.to_be_bytes());
    }

    /// Appends a big-endian `i32`.
    pub fn write_i32(&mut self, value: i32) {
        self.buffer.extend_from_slice(&value.to_be_bytes());
    }

    /// Appends a big-endian `i64`.
    pub fn write_i64(&mut self, value: i64) {
        self.buffer.extend_from_slice(&value.to_be_bytes());
    }

    /// Appends a big-endian 32-bit float.
    pub fn write_f32(&mut self, value: f32) {
        self.buffer.extend_from_slice(&value.to_be_bytes());
    }

    /// Appends a big-endian 64-bit float.
    pub fn write_f64(&mut self, value: f64) {
        self.buffer.extend_from_slice(&value.to_be_bytes());
    }

    /// Appends a packed block position.
    pub fn write_position(&mut self, value: Position) {
        let packed = ((i64::from(value.x) & 0x3FFFFFF) << 38)
            | ((i64::from(value.z) & 0x3FFFFFF) << 12)
            | (i64::from(value.y) & 0xFFF);
        self.buffer.extend_from_slice(&packed.to_be_bytes());
    }

    /// Appends a boolean as a single byte.
    pub fn write_bool(&mut self, value: bool) {
        self.buffer.push(u8::from(value));
    }

    /// Appends a 16-byte UUID.
    pub fn write_uuid(&mut self, value: Uuid) {
        self.buffer.extend_from_slice(&value);
    }

    /// Appends a network NBT value (type byte plus payload, no root name).
    pub fn write_nbt(&mut self, tag: &crate::nbt::NbtTag) -> Result<(), CodecError> {
        tag.encode_network(&mut self.buffer)?;
        Ok(())
    }

    /// Returns the written bytes without consuming the writer.
    pub fn as_bytes(&self) -> &[u8] {
        &self.buffer
    }

    /// Consumes the writer and returns the written bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Values and encodings from the public protocol documentation.
    const KNOWN_VARINTS: &[(i32, &[u8])] = &[
        (0, &[0x00]),
        (1, &[0x01]),
        (2, &[0x02]),
        (127, &[0x7F]),
        (128, &[0x80, 0x01]),
        (255, &[0xFF, 0x01]),
        (25565, &[0xDD, 0xC7, 0x01]),
        (2097151, &[0xFF, 0xFF, 0x7F]),
        (2147483647, &[0xFF, 0xFF, 0xFF, 0xFF, 0x07]),
        (-1, &[0xFF, 0xFF, 0xFF, 0xFF, 0x0F]),
        (-2147483648, &[0x80, 0x80, 0x80, 0x80, 0x08]),
    ];

    #[test]
    fn varint_matches_documented_encodings() {
        for &(value, expected) in KNOWN_VARINTS {
            let mut encoded = Vec::new();
            encode_varint(value, &mut encoded);
            assert_eq!(encoded, expected, "encoding of {value}");

            let (decoded, consumed) = decode_varint(expected).expect("decodable");
            assert_eq!(decoded, value, "decoding of {value}");
            assert_eq!(consumed, expected.len(), "consumed bytes for {value}");
        }
    }

    #[test]
    fn varint_roundtrips_across_the_range() {
        let samples = [
            i32::MIN,
            -70000,
            -25565,
            -128,
            -1,
            0,
            1,
            127,
            128,
            300,
            25565,
            2097151,
            2147483647,
        ];
        for value in samples {
            let mut encoded = Vec::new();
            encode_varint(value, &mut encoded);
            let (decoded, consumed) = decode_varint(&encoded).expect("decodable");
            assert_eq!(decoded, value, "roundtrip of {value}");
            assert_eq!(consumed, encoded.len(), "consumed bytes for {value}");
        }
    }

    #[test]
    fn varint_rejects_truncated_input() {
        let err = decode_varint(&[0x80]).expect_err("truncated");
        assert!(matches!(err, CodecError::UnexpectedEof));
    }

    #[test]
    fn varint_rejects_overlong_input() {
        let err = decode_varint(&[0x80, 0x80, 0x80, 0x80, 0x80]).expect_err("too long");
        assert!(matches!(err, CodecError::VarIntTooLong));
    }

    #[test]
    fn varint_accepts_non_canonical_encoding() {
        // `81 00` is an unnecessarily long but valid encoding of 1.
        let (value, consumed) = decode_varint(&[0x81, 0x00]).expect("decodable");
        assert_eq!(value, 1);
        assert_eq!(consumed, 2);
    }

    #[test]
    fn string_roundtrips_unicode() {
        for text in ["", "localhost", "Привет, мир!", "emoji: 🌍"] {
            let mut writer = PacketWriter::new();
            writer
                .write_string(text, MAX_STRING_CHARS)
                .expect("encodable");
            let mut reader = PacketReader::new(writer.as_bytes());
            let decoded = reader.read_string(MAX_STRING_CHARS).expect("decodable");
            assert_eq!(decoded, text);
            reader.ensure_empty().expect("no trailing bytes");
        }
    }

    #[test]
    fn string_length_counts_utf16_code_units() {
        // 🌍 is one scalar value but two UTF-16 code units.
        let mut writer = PacketWriter::new();
        let err = writer.write_string("🌍🌍", 3).expect_err("too long");
        assert!(matches!(
            err,
            CodecError::StringTooLong {
                length: 4,
                limit: 3
            }
        ));
    }

    #[test]
    fn string_rejects_invalid_utf8() {
        // Length prefix 3 followed by invalid UTF-8 bytes.
        let bytes = [0x03, 0xFF, 0xFE, 0xFD];
        let mut reader = PacketReader::new(&bytes);
        let err = reader.read_string(MAX_STRING_CHARS).expect_err("invalid");
        assert!(matches!(err, CodecError::InvalidUtf8(_)));
    }

    #[test]
    fn reader_rejects_negative_length_prefix() {
        // -1 as a VarInt: five bytes, all ones in the value.
        let bytes = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
        let mut reader = PacketReader::new(&bytes);
        let err = reader.read_string(MAX_STRING_CHARS).expect_err("negative");
        assert!(matches!(err, CodecError::NegativeLength(-1)));
    }

    #[test]
    fn fixed_width_integers_are_big_endian() {
        let mut writer = PacketWriter::new();
        writer.write_u16(0x1234);
        writer.write_i64(-2);
        assert_eq!(
            writer.as_bytes(),
            &[0x12, 0x34, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE]
        );

        let mut reader = PacketReader::new(writer.as_bytes());
        assert_eq!(reader.read_u16().expect("u16"), 0x1234);
        assert_eq!(reader.read_i64().expect("i64"), -2);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn ints_and_floats_are_big_endian() {
        let mut writer = PacketWriter::new();
        writer.write_i32(-2);
        writer.write_f32(1.5);
        writer.write_f64(-0.5);
        assert_eq!(
            writer.as_bytes(),
            &[
                0xFF, 0xFF, 0xFF, 0xFE, // -2
                0x3F, 0xC0, 0x00, 0x00, // 1.5f
                0xBF, 0xE0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // -0.5
            ]
        );

        let mut reader = PacketReader::new(writer.as_bytes());
        assert_eq!(reader.read_i32().expect("i32"), -2);
        assert_eq!(reader.read_f32().expect("f32"), 1.5);
        assert_eq!(reader.read_f64().expect("f64"), -0.5);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn positions_pack_x_z_and_y() {
        // Known packing: x as 26 bits, then z (26), then y (12), big-endian.
        let mut writer = PacketWriter::new();
        writer.write_position(Position { x: 1, y: 2, z: 3 });
        // x=1 occupies bits 38-63 (1 << 38 = 0x40_0000_0000), z=3 bits
        // 12-37, y=2 the low 12 bits.
        assert_eq!(
            writer.as_bytes(),
            &[0x00, 0x00, 0x00, 0x40, 0x00, 0x00, 0x30, 0x02]
        );

        // Example from the public protocol documentation: a 64-bit position
        // of x = 18357644, z = -20882616, y = 831.
        let mut writer = PacketWriter::new();
        writer.write_position(Position {
            x: 18_357_644,
            y: 831,
            z: -20_882_616,
        });
        assert_eq!(
            writer.as_bytes(),
            &[0x46, 0x07, 0x63, 0x2C, 0x15, 0xB4, 0x83, 0x3F]
        );

        for position in [
            Position::default(),
            Position {
                x: -1,
                y: -64,
                z: 30_000_000,
            },
            Position {
                x: 30_000_000,
                y: 319,
                z: -30_000_000,
            },
        ] {
            let mut writer = PacketWriter::new();
            writer.write_position(position);
            let mut reader = PacketReader::new(writer.as_bytes());
            assert_eq!(reader.read_position().expect("position"), position);
            reader.ensure_empty().expect("no trailing bytes");
        }
    }

    #[test]
    fn reader_reports_trailing_bytes() {
        let reader = PacketReader::new(&[0x01, 0x02]);
        let err = reader.ensure_empty().expect_err("trailing");
        assert!(matches!(err, CodecError::TrailingBytes { remaining: 2 }));
    }

    #[test]
    fn nbt_roundtrips_through_the_packet_cursor() {
        use crate::nbt::NbtTag;
        use std::collections::BTreeMap;

        let tag = NbtTag::Compound(BTreeMap::from([
            ("height".to_owned(), NbtTag::Short(384)),
            ("name".to_owned(), NbtTag::String("overworld".to_owned())),
        ]));

        let mut writer = PacketWriter::new();
        writer.write_bool(true);
        writer.write_nbt(&tag).expect("encodable");
        writer.write_u8(7);
        assert_eq!(writer.as_bytes().last(), Some(&7));

        let mut reader = PacketReader::new(writer.as_bytes());
        assert!(reader.read_bool().expect("bool"));
        assert_eq!(reader.read_nbt().expect("nbt"), tag);
        assert_eq!(reader.read_u8().expect("u8"), 7);
        reader.ensure_empty().expect("no trailing bytes");
    }
}
