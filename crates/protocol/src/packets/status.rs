//! Status state packets: server list request/response and ping/pong.

use crate::codec::{CodecError, PacketReader, PacketWriter, MAX_STRING_CHARS};
use crate::frame::encode_frame;

/// Serverbound status request; it has no fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusRequest;

impl StatusRequest {
    /// Packet id of the status request (status state, serverbound).
    pub const ID: u32 = 0x00;

    /// Encodes the payload; there is none, so this always succeeds.
    pub fn encode_payload(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Decodes the payload and fails if any bytes remain.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        reader.ensure_empty()?;
        Ok(Self)
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Clientbound status response carrying the server list JSON document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusResponse {
    /// JSON document describing the server; see [`crate::StatusInfo::to_json`].
    pub json: String,
}

impl StatusResponse {
    /// Packet id of the status response (status state, clientbound).
    pub const ID: u32 = 0x00;

    /// Encodes the payload (the JSON as a protocol string).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_string(&self.json, MAX_STRING_CHARS)?;
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            json: reader.read_string(MAX_STRING_CHARS)?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload()?)
    }
}

/// Serverbound ping request with an opaque payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PingRequest {
    /// Opaque payload; Notchian clients send a timestamp in milliseconds.
    pub payload: i64,
}

impl PingRequest {
    /// Packet id of the ping request (status state, serverbound).
    pub const ID: u32 = 0x01;

    /// Encodes the payload (a big-endian `i64`).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_i64(self.payload);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            payload: reader.read_i64()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Clientbound pong response echoing the ping payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PongResponse {
    /// Payload echoed back from the ping request.
    pub payload: i64,
}

impl PongResponse {
    /// Packet id of the pong response (status state, clientbound).
    pub const ID: u32 = 0x01;

    /// Encodes the payload (a big-endian `i64`).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_i64(self.payload);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            payload: reader.read_i64()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::split_frame;

    #[test]
    fn status_request_frame_has_no_fields() {
        let frame = StatusRequest.to_frame().expect("encodable");
        assert_eq!(frame, vec![0x01, StatusRequest::ID as u8]);

        let (id, mut reader) = split_frame(&frame).expect("splittable");
        assert_eq!(id, StatusRequest::ID);
        StatusRequest::decode_payload(&mut reader).expect("decodable");
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn status_response_roundtrips_unicode_json() {
        let response = StatusResponse {
            json: r#"{"description":{"text":"Привет 🌍"}}"#.to_owned(),
        };
        let frame = response.to_frame().expect("encodable");
        let (id, mut reader) = split_frame(&frame).expect("splittable");
        assert_eq!(id, StatusResponse::ID);

        let decoded = StatusResponse::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, response);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn status_response_rejects_oversized_json() {
        let response = StatusResponse {
            json: "x".repeat(MAX_STRING_CHARS + 1),
        };
        let err = response.encode_payload().expect_err("too long");
        assert!(matches!(err, CodecError::StringTooLong { .. }));
    }

    #[test]
    fn ping_and_pong_roundtrip_payloads() {
        for payload in [0_i64, -1, 1_234_567_890, i64::MIN, i64::MAX] {
            let ping = PingRequest { payload };
            let frame = ping.to_frame().expect("encodable");
            let (id, mut reader) = split_frame(&frame).expect("splittable");
            assert_eq!(id, PingRequest::ID);
            let decoded = PingRequest::decode_payload(&mut reader).expect("decodable");
            assert_eq!(decoded.payload, payload);
            reader.ensure_empty().expect("no trailing bytes");

            let pong = PongResponse { payload };
            let frame = pong.to_frame().expect("encodable");
            let (id, mut reader) = split_frame(&frame).expect("splittable");
            assert_eq!(id, PongResponse::ID);
            let decoded = PongResponse::decode_payload(&mut reader).expect("decodable");
            assert_eq!(decoded.payload, payload);
            reader.ensure_empty().expect("no trailing bytes");
        }
    }
}
