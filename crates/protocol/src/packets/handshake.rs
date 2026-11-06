//! Handshake packet (state: handshaking, serverbound).

use crate::codec::{CodecError, PacketReader, PacketWriter};
use crate::frame::encode_frame;

/// Sanity limit for the handshake server address, in UTF-16 code units.
///
/// This is a server-side bound on untrusted input; the protocol itself defines
/// the field as a context-limited string.
pub const MAX_SERVER_ADDRESS_CHARS: usize = 255;

/// Target state requested by the client after the handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NextState {
    /// Server list ping (`status` state).
    Status = 1,
    /// Login sequence (`login` state).
    Login = 2,
    /// Server transfer (`transfer` state).
    Transfer = 3,
}

impl NextState {
    /// Converts a wire value into a [`NextState`].
    pub fn from_i32(value: i32) -> Result<Self, CodecError> {
        match value {
            1 => Ok(Self::Status),
            2 => Ok(Self::Login),
            3 => Ok(Self::Transfer),
            other => Err(CodecError::InvalidNextState(other)),
        }
    }
}

/// Serverbound handshake packet: the first packet of every connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handshake {
    /// Protocol version the client intends to use.
    pub protocol_version: i32,
    /// Hostname or IP the client believes it connected to.
    pub server_address: String,
    /// Port the client believes it connected to.
    pub server_port: u16,
    /// State to switch to after this packet.
    pub next_state: NextState,
}

impl Handshake {
    /// Packet id of the handshake packet.
    pub const ID: u32 = 0x00;

    /// Encodes the packet payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.protocol_version);
        writer.write_string(&self.server_address, MAX_SERVER_ADDRESS_CHARS)?;
        writer.write_u16(self.server_port);
        writer.write_varint(self.next_state as i32);
        Ok(writer.into_bytes())
    }

    /// Decodes the packet payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            protocol_version: reader.read_varint()?,
            server_address: reader.read_string(MAX_SERVER_ADDRESS_CHARS)?,
            server_port: reader.read_u16()?,
            next_state: NextState::from_i32(reader.read_varint()?)?,
        })
    }

    /// Encodes the packet as a complete frame (length prefix + id + payload).
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::split_frame;

    fn sample_handshake() -> Handshake {
        Handshake {
            protocol_version: 1234,
            server_address: "localhost".to_owned(),
            server_port: 25565,
            next_state: NextState::Status,
        }
    }

    #[test]
    fn handshake_roundtrips_through_a_frame() {
        let handshake = sample_handshake();
        let frame = handshake.to_frame().expect("encodable");
        let (id, mut reader) = split_frame(&frame).expect("splittable");
        assert_eq!(id, Handshake::ID);

        let decoded = Handshake::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, handshake);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn handshake_wire_layout_is_length_prefixed() {
        let mut handshake = sample_handshake();
        handshake.protocol_version = 1;
        handshake.server_port = 25565;
        let frame = handshake.to_frame().expect("encodable");
        // Handshake payload: VarInt(1) + String("localhost") + u16 + VarInt(1).
        // "localhost" is 9 bytes, so payload = 1 + 1 + 9 + 2 + 1 = 14.
        assert_eq!(frame[0], 14 + 1, "length covers id + payload");
        assert_eq!(frame[1], Handshake::ID as u8, "packet id");
    }

    #[test]
    fn all_next_states_roundtrip() {
        for next_state in [NextState::Status, NextState::Login, NextState::Transfer] {
            let mut handshake = sample_handshake();
            handshake.next_state = next_state;
            let frame = handshake.to_frame().expect("encodable");
            let (id, mut reader) = split_frame(&frame).expect("splittable");
            assert_eq!(id, Handshake::ID);
            let decoded = Handshake::decode_payload(&mut reader).expect("decodable");
            assert_eq!(decoded.next_state, next_state);
        }
    }

    #[test]
    fn unknown_next_state_is_rejected() {
        let mut writer = PacketWriter::new();
        writer.write_varint(1);
        writer
            .write_string("localhost", MAX_SERVER_ADDRESS_CHARS)
            .expect("encodable");
        writer.write_u16(25565);
        writer.write_varint(7);

        let mut reader = PacketReader::new(writer.as_bytes());
        let err = Handshake::decode_payload(&mut reader).expect_err("invalid state");
        assert!(matches!(err, CodecError::InvalidNextState(7)));
    }

    #[test]
    fn overlong_server_address_is_rejected() {
        let mut handshake = sample_handshake();
        handshake.server_address = "a".repeat(MAX_SERVER_ADDRESS_CHARS + 1);
        let err = handshake.encode_payload().expect_err("too long");
        assert!(matches!(err, CodecError::StringTooLong { .. }));
    }
}
