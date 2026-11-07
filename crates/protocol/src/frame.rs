//! Packet framing: `Length (VarInt) + Packet ID (VarInt) + Data`.
//!
//! The protocol caps a packet (Packet ID + Data) at `2^21 - 1` bytes and
//! requires the length field itself to be at most 3 bytes long.

use crate::codec::{decode_varint, encode_varint, CodecError, PacketReader};

/// Maximum size of a packet body (Packet ID + Data), in bytes.
///
/// From the protocol documentation: packets cannot be larger than `2^21 - 1`
/// (2097151) bytes, the maximum representable by a 3-byte VarInt.
// SOURCE: wiki, unverified: the cap is documented, never compared against a
// vanilla server's behaviour (the parity crate covers it).
pub const MAX_FRAME_LEN: usize = 2097151;

/// Maximum encoded size of the length field itself, in bytes.
const MAX_FRAME_LEN_BYTES: usize = 3;

/// Builds a complete packet frame: length prefix, packet id and payload.
pub fn encode_frame(packet_id: u32, payload: &[u8]) -> Result<Vec<u8>, CodecError> {
    let packet_id =
        i32::try_from(packet_id).map_err(|_| CodecError::InvalidPacketId(packet_id as i32))?;
    let mut id_bytes = Vec::new();
    encode_varint(packet_id, &mut id_bytes);

    let length = id_bytes
        .len()
        .checked_add(payload.len())
        .ok_or(CodecError::FrameTooLong {
            length: usize::MAX,
            limit: MAX_FRAME_LEN,
        })?;
    if length > MAX_FRAME_LEN {
        return Err(CodecError::FrameTooLong {
            length,
            limit: MAX_FRAME_LEN,
        });
    }

    let mut frame = Vec::with_capacity(length + MAX_FRAME_LEN_BYTES);
    encode_varint(length as i32, &mut frame);
    frame.extend_from_slice(&id_bytes);
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Splits a frame body (everything after the length prefix) into the packet id
/// and a reader positioned at the packet payload.
pub fn decode_frame(body: &[u8]) -> Result<(u32, PacketReader<'_>), CodecError> {
    let mut reader = PacketReader::new(body);
    let id = reader.read_varint()?;
    let id = u32::try_from(id).map_err(|_| CodecError::InvalidPacketId(id))?;
    Ok((id, reader))
}

/// Splits a complete encoded frame (length prefix included) into the packet id
/// and a reader positioned at the packet payload.
///
/// The frame must contain exactly one packet.
pub fn split_frame(frame: &[u8]) -> Result<(u32, PacketReader<'_>), CodecError> {
    let (length, consumed) = decode_varint(frame)?;
    let length = check_frame_len(length, consumed)?;
    let end = consumed
        .checked_add(length)
        .ok_or(CodecError::UnexpectedEof)?;
    let body = frame.get(consumed..end).ok_or(CodecError::UnexpectedEof)?;
    if frame.len() > end {
        return Err(CodecError::TrailingBytes {
            remaining: frame.len() - end,
        });
    }
    decode_frame(body)
}

/// Validates a frame length value read from the wire.
///
/// The protocol requires the length field to fit into 3 bytes, which also caps
/// the value at [`MAX_FRAME_LEN`].
pub fn check_frame_len(length: i32, encoded_in: usize) -> Result<usize, CodecError> {
    if length < 0 {
        return Err(CodecError::NegativeLength(length));
    }
    if encoded_in > MAX_FRAME_LEN_BYTES {
        return Err(CodecError::FrameLengthEncodingTooLong);
    }
    Ok(length as usize)
}

/// Reads a length prefix from `bytes` (used when the caller has already
/// buffered the encoded VarInt).
pub fn decode_frame_len(bytes: &[u8]) -> Result<usize, CodecError> {
    let (length, consumed) = decode_varint(bytes)?;
    check_frame_len(length, consumed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip() {
        let frame = encode_frame(0x00, &[0x01, 0x02, 0x03]).expect("encodable");
        // Length = 1 (id) + 3 (payload) = 4.
        assert_eq!(frame, vec![0x04, 0x00, 0x01, 0x02, 0x03]);

        let (id, mut payload) = decode_frame(&frame[1..]).expect("decodable");
        assert_eq!(id, 0x00);
        assert_eq!(payload.read_varint().expect("payload"), 0x01);
        assert_eq!(payload.read_varint().expect("payload"), 0x02);
        assert_eq!(payload.read_varint().expect("payload"), 0x03);
        payload.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn empty_payload_frame() {
        let frame = encode_frame(0x01, &[]).expect("encodable");
        assert_eq!(frame, vec![0x01, 0x01]);
        let (id, payload) = decode_frame(&frame[1..]).expect("decodable");
        assert_eq!(id, 0x01);
        payload.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn oversized_frame_is_rejected() {
        let payload = vec![0u8; MAX_FRAME_LEN];
        let err = encode_frame(0x00, &payload).expect_err("too big");
        assert!(matches!(
            err,
            CodecError::FrameTooLong {
                length,
                limit: MAX_FRAME_LEN
            } if length == MAX_FRAME_LEN + 1
        ));
    }

    #[test]
    fn length_field_over_three_bytes_is_rejected() {
        let err = check_frame_len(1, 4).expect_err("field too long");
        assert!(matches!(err, CodecError::FrameLengthEncodingTooLong));
    }

    #[test]
    fn negative_frame_length_is_rejected() {
        let err = check_frame_len(-1, 5).expect_err("negative");
        assert!(matches!(err, CodecError::NegativeLength(-1)));
    }

    #[test]
    fn maximum_frame_length_is_accepted() {
        let length = check_frame_len(MAX_FRAME_LEN as i32, 3).expect("valid");
        assert_eq!(length, MAX_FRAME_LEN);
    }

    #[test]
    fn negative_packet_id_is_rejected() {
        // A packet id of -1 cannot be represented as a non-negative id.
        let err = encode_frame(u32::MAX, &[]).expect_err("invalid id");
        assert!(matches!(err, CodecError::InvalidPacketId(-1)));
    }

    #[test]
    fn split_frame_consumes_length_prefix() {
        let frame = encode_frame(0x02, &[0x2A]).expect("encodable");
        let (id, mut payload) = split_frame(&frame).expect("splittable");
        assert_eq!(id, 0x02);
        assert_eq!(payload.read_varint().expect("payload"), 42);
        payload.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn split_frame_rejects_trailing_bytes() {
        let mut frame = encode_frame(0x00, &[]).expect("encodable");
        frame.push(0x00);
        let err = split_frame(&frame).expect_err("trailing");
        assert!(matches!(err, CodecError::TrailingBytes { remaining: 1 }));
    }
}
