//! Login state packets: start, compression, success and acknowledgement.
//!
//! The offline login flow used by this server skips encryption entirely:
//! Login Start is answered with Set Compression (optional) and Login
//! Success, and the client confirms with Login Acknowledged to enter the
//! configuration state.

use crate::codec::{CodecError, PacketReader, PacketWriter, Uuid, MAX_STRING_CHARS};
use crate::frame::encode_frame;

/// Maximum player name length in UTF-16 code units (protocol String (16)).
const MAX_NAME_CHARS: usize = 16;

/// Maximum length of a profile property name (protocol String (64)).
const MAX_PROPERTY_NAME_CHARS: usize = 64;

/// Maximum length of a profile property signature (protocol String (1024)).
const MAX_SIGNATURE_CHARS: usize = 1024;

/// Serverbound Login Start: the first packet of the login state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginStart {
    /// Player username.
    pub name: String,
    /// UUID the client believes is its own. The vanilla server ignores this
    /// field and derives its own offline UUID from the name.
    pub uuid: Uuid,
}

impl LoginStart {
    /// Packet id of Login Start (login state, serverbound).
    pub const ID: u32 = 0x00;

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_string(&self.name, MAX_NAME_CHARS)?;
        writer.write_uuid(self.uuid);
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            name: reader.read_string(MAX_NAME_CHARS)?,
            uuid: reader.read_uuid()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload()?)
    }
}

/// Clientbound Set Compression: switches every following packet to the
/// compressed packet format. The vanilla server omits the packet when
/// compression is disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetCompression {
    /// Packets at or above this size in bytes are compressed; a negative
    /// threshold disables compression again.
    pub threshold: i32,
}

impl SetCompression {
    /// Packet id of Set Compression (login state, clientbound).
    pub const ID: u32 = 0x03;

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.threshold);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            threshold: reader.read_varint()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// A player profile as carried by Login Success.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GameProfile {
    /// Profile UUID.
    pub uuid: Uuid,
    /// Profile username.
    pub name: String,
    /// Profile properties; an empty list makes the client use the default
    /// skin for the UUID.
    pub properties: Vec<ProfileProperty>,
}

/// One profile property, usually `textures` with a Base64 value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileProperty {
    /// Property name, e.g. `textures`.
    pub name: String,
    /// Property value.
    pub value: String,
    /// Mojang signature over the value, when present.
    pub signature: Option<String>,
}

/// Clientbound Login Success: the client answers with Login Acknowledged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginSuccess {
    /// Profile the client adopts for this session.
    pub profile: GameProfile,
    /// Session UUID reported to the client for this connection.
    pub session_id: Uuid,
}

impl LoginSuccess {
    /// Packet id of Login Success (login state, clientbound).
    pub const ID: u32 = 0x02;

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_uuid(self.profile.uuid);
        writer.write_string(&self.profile.name, MAX_NAME_CHARS)?;
        writer.write_varint(self.profile.properties.len() as i32);
        for property in &self.profile.properties {
            writer.write_string(&property.name, MAX_PROPERTY_NAME_CHARS)?;
            writer.write_string(&property.value, MAX_STRING_CHARS)?;
            match &property.signature {
                Some(signature) => {
                    writer.write_bool(true);
                    writer.write_string(signature, MAX_SIGNATURE_CHARS)?;
                }
                None => writer.write_bool(false),
            }
        }
        writer.write_uuid(self.session_id);
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        let uuid = reader.read_uuid()?;
        let name = reader.read_string(MAX_NAME_CHARS)?;
        let property_count = reader.read_varint()?;
        let property_count = usize::try_from(property_count)
            .map_err(|_| CodecError::NegativeLength(property_count))?;
        let mut properties = Vec::with_capacity(property_count.min(16));
        for _ in 0..property_count {
            let property_name = reader.read_string(MAX_PROPERTY_NAME_CHARS)?;
            let value = reader.read_string(MAX_STRING_CHARS)?;
            let signature = if reader.read_bool()? {
                Some(reader.read_string(MAX_SIGNATURE_CHARS)?)
            } else {
                None
            };
            properties.push(ProfileProperty {
                name: property_name,
                value,
                signature,
            });
        }
        let session_id = reader.read_uuid()?;
        Ok(Self {
            profile: GameProfile {
                uuid,
                name,
                properties,
            },
            session_id,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload()?)
    }
}

/// Serverbound Login Acknowledged: confirms Login Success and switches the
/// connection to the configuration state. It has no fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoginAcknowledged;

impl LoginAcknowledged {
    /// Packet id of Login Acknowledged (login state, serverbound).
    pub const ID: u32 = 0x03;

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

/// Clientbound Disconnect (login): ends the connection before Login Success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectLogin {
    /// JSON-encoded text component explaining the disconnect.
    pub reason: String,
}

impl DisconnectLogin {
    /// Packet id of Disconnect (login state, clientbound).
    pub const ID: u32 = 0x00;

    /// Builds a disconnect reason from plain text.
    pub fn from_text(text: &str) -> Self {
        Self {
            reason: serde_json::json!({ "text": text }).to_string(),
        }
    }

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_string(&self.reason, MAX_STRING_CHARS)?;
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            reason: reader.read_string(MAX_STRING_CHARS)?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::split_frame;

    const SAMPLE_UUID: Uuid = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10,
    ];

    #[test]
    fn login_start_roundtrips_name_and_uuid() {
        let packet = LoginStart {
            name: "Steve".to_owned(),
            uuid: SAMPLE_UUID,
        };
        let frame = packet.to_frame().expect("encodable");
        let (id, mut reader) = split_frame(&frame).expect("splittable");
        assert_eq!(id, LoginStart::ID);

        let decoded = LoginStart::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn login_start_rejects_an_oversized_name() {
        let packet = LoginStart {
            name: "x".repeat(MAX_NAME_CHARS + 1),
            uuid: SAMPLE_UUID,
        };
        let err = packet.encode_payload().expect_err("too long");
        assert!(matches!(err, CodecError::StringTooLong { .. }));
    }

    #[test]
    fn set_compression_carries_a_varint_threshold() {
        for threshold in [256, 0, -1] {
            let packet = SetCompression { threshold };
            let frame = packet.to_frame().expect("encodable");
            let (id, mut reader) = split_frame(&frame).expect("splittable");
            assert_eq!(id, SetCompression::ID);

            let decoded = SetCompression::decode_payload(&mut reader).expect("decodable");
            assert_eq!(decoded, packet);
            reader.ensure_empty().expect("no trailing bytes");
        }
    }

    #[test]
    fn login_success_roundtrips_a_profile_with_properties() {
        let packet = LoginSuccess {
            profile: GameProfile {
                uuid: SAMPLE_UUID,
                name: "Steve".to_owned(),
                properties: vec![ProfileProperty {
                    name: "textures".to_owned(),
                    value: "eyJ2YWx1ZSI6ICJ0ZXh0In0=".to_owned(),
                    signature: Some("c2lnbmF0dXJl".to_owned()),
                }],
            },
            session_id: [0xAA; 16],
        };
        let frame = packet.to_frame().expect("encodable");
        let (id, mut reader) = split_frame(&frame).expect("splittable");
        assert_eq!(id, LoginSuccess::ID);

        let decoded = LoginSuccess::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn login_success_roundtrips_an_empty_property_list() {
        let packet = LoginSuccess {
            profile: GameProfile {
                uuid: [0; 16],
                name: "Alex".to_owned(),
                properties: Vec::new(),
            },
            session_id: [0xBB; 16],
        };
        let frame = packet.to_frame().expect("encodable");
        let (id, mut reader) = split_frame(&frame).expect("splittable");
        assert_eq!(id, LoginSuccess::ID);

        let decoded = LoginSuccess::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn login_success_places_the_session_id_last() {
        let packet = LoginSuccess {
            profile: GameProfile {
                uuid: [1; 16],
                name: "A".to_owned(),
                properties: Vec::new(),
            },
            session_id: [2; 16],
        };
        let payload = packet.encode_payload().expect("encodable");
        // UUID (16) + name (1 + 1) + property count (1) + session id (16).
        assert_eq!(payload.len(), 16 + 2 + 1 + 16);
        assert_eq!(&payload[16 + 2 + 1..], &[2; 16]);
    }

    #[test]
    fn login_acknowledged_has_no_fields() {
        let frame = LoginAcknowledged.to_frame().expect("encodable");
        assert_eq!(frame, vec![0x01, LoginAcknowledged::ID as u8]);

        let (id, mut reader) = split_frame(&frame).expect("splittable");
        assert_eq!(id, LoginAcknowledged::ID);
        LoginAcknowledged::decode_payload(&mut reader).expect("decodable");
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn disconnect_reason_carries_a_json_text_component() {
        let packet = DisconnectLogin::from_text("Server is full");
        let parsed: serde_json::Value = serde_json::from_str(&packet.reason).expect("valid json");
        assert_eq!(parsed["text"], "Server is full");

        let frame = packet.to_frame().expect("encodable");
        let (id, mut reader) = split_frame(&frame).expect("splittable");
        assert_eq!(id, DisconnectLogin::ID);

        let decoded = DisconnectLogin::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }
}
