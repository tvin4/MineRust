//! Clientbound entity state packets of the M4.5 visibility stream: metadata,
//! attributes and an absolute position sync.
//!
//! Every layout is taken from the two-bot vanilla trace (scenarios С1–С4,
//! `docs/ARCH-M4.md` §6) and each codec's test reproduces a recorded payload
//! byte for byte. Fields whose meaning the trace does not establish are
//! carried explicitly and marked «не установлено».
//!
//! SOURCE: two-bot vanilla trace of 26.3 (byte layout observed), not wiki.

use super::packet_impl::play_packet;
use super::read_count;
use crate::codec::{CodecError, PacketReader, PacketWriter, Uuid};

/// Set Entity Data: an entity's metadata entries.
///
/// The trace only shows the standing bot's value, so the entry bytes are
/// carried opaquely: `entity id`, raw metadata entries, then the `0xFF`
/// terminator. The entry contents are «не установлено»; the observed values
/// are recorded in `docs/ARCH-M4.md` §6 and the tests.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetEntityData {
    /// Entity id.
    pub entity_id: i32,
    /// Metadata entries exactly as recorded, without the `0xFF` terminator.
    pub metadata: Vec<u8>,
}

impl SetEntityData {
    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.entity_id);
        writer.write_bytes(&self.metadata);
        writer.write_u8(0xFF);
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        let entity_id = reader.read_varint()?;
        let remaining = reader.remaining();
        if remaining == 0 {
            return Err(CodecError::UnexpectedEof);
        }
        let mut metadata = reader.read_bytes(remaining)?.to_vec();
        if metadata.pop() != Some(0xFF) {
            return Err(CodecError::Unsupported {
                context: "set_entity_data terminator",
            });
        }
        Ok(Self {
            entity_id,
            metadata,
        })
    }
}

play_packet!(SetEntityData, 0x65, "minecraft:set_entity_data");

/// One attribute modifier of [`UpdateAttributes`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttributeModifier {
    /// Modifier UUID.
    pub id: Uuid,
    /// Modifier amount.
    pub amount: f64,
    /// Operation id.
    pub operation: u8,
}

/// One attribute of [`UpdateAttributes`].
#[derive(Debug, Clone, PartialEq)]
pub struct AttributeEntry {
    /// Attribute key id (8, 26 and 13 in the trace).
    pub key: i32,
    /// Base value.
    pub base: f64,
    /// Modifiers; none were recorded.
    pub modifiers: Vec<AttributeModifier>,
}

/// Update Attributes: an entity's base attributes and modifiers.
///
/// Observed payload (len 32): `entity id`, `VarInt` count, then per attribute
/// key `VarInt`, `f64` base, `VarInt` modifier count and the modifiers.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UpdateAttributes {
    /// Entity id.
    pub entity_id: i32,
    /// Attributes in wire order.
    pub attributes: Vec<AttributeEntry>,
}

impl UpdateAttributes {
    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.entity_id);
        writer.write_varint(self.attributes.len() as i32);
        for attribute in &self.attributes {
            writer.write_varint(attribute.key);
            writer.write_f64(attribute.base);
            writer.write_varint(attribute.modifiers.len() as i32);
            for modifier in &attribute.modifiers {
                writer.write_uuid(modifier.id);
                writer.write_f64(modifier.amount);
                writer.write_u8(modifier.operation);
            }
        }
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        let entity_id = reader.read_varint()?;
        let count = read_count(reader)?;
        let mut attributes = Vec::with_capacity(count.min(16));
        for _ in 0..count {
            let key = reader.read_varint()?;
            let base = reader.read_f64()?;
            let modifier_count = read_count(reader)?;
            let mut modifiers = Vec::with_capacity(modifier_count.min(16));
            for _ in 0..modifier_count {
                modifiers.push(AttributeModifier {
                    id: reader.read_uuid()?,
                    amount: reader.read_f64()?,
                    operation: reader.read_u8()?,
                });
            }
            attributes.push(AttributeEntry {
                key,
                base,
                modifiers,
            });
        }
        Ok(Self {
            entity_id,
            attributes,
        })
    }
}

play_packet!(UpdateAttributes, 0x86, "minecraft:update_attributes");

/// Entity Position Sync: an absolute position and rotation update.
///
/// Observed payload (len 37): `entity id`, two `VarInt` header fields,
/// three `f64` coordinates, a `VarInt` rotation field, yaw and pitch `f32`
/// and one trailing byte. For the traced player the header is `01 01`, the
/// rotation field is `02` and the trailing byte is `00` or `01`; mobs use
/// `03` as the rotation field. All three are «не установлено» in meaning.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityPositionSync {
    /// Entity id.
    pub entity_id: i32,
    /// First header field (1 in the trace).
    pub teleport_id: i32,
    /// Second header field (1 in the trace).
    pub flags: i32,
    /// Absolute X coordinate.
    pub x: f64,
    /// Absolute Y coordinate.
    pub y: f64,
    /// Absolute Z coordinate.
    pub z: f64,
    /// Rotation field (2 for the traced player, 3 for mobs).
    pub rotation: i32,
    /// Absolute yaw in degrees.
    pub yaw: f32,
    /// Absolute pitch in degrees.
    pub pitch: f32,
    /// Trailing byte; 0 or 1 in the trace, meaning «не установлено».
    pub tail: bool,
}

impl EntityPositionSync {
    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.entity_id);
        writer.write_varint(self.teleport_id);
        writer.write_varint(self.flags);
        writer.write_f64(self.x);
        writer.write_f64(self.y);
        writer.write_f64(self.z);
        writer.write_varint(self.rotation);
        writer.write_f32(self.yaw);
        writer.write_f32(self.pitch);
        writer.write_bool(self.tail);
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            entity_id: reader.read_varint()?,
            teleport_id: reader.read_varint()?,
            flags: reader.read_varint()?,
            x: reader.read_f64()?,
            y: reader.read_f64()?,
            z: reader.read_f64()?,
            rotation: reader.read_varint()?,
            yaw: reader.read_f32()?,
            pitch: reader.read_f32()?,
            tail: reader.read_bool()?,
        })
    }
}

play_packet!(EntityPositionSync, 0x23, "minecraft:entity_position_sync");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::test_support::{check, hex};

    #[test]
    fn set_entity_data_reproduces_the_standing_bot_values() {
        check(
            SetEntityData {
                entity_id: 77,
                metadata: hex("090341a0000010007f"),
            },
            "4d090341a0000010007fff",
            SetEntityData::encode_payload,
            SetEntityData::decode_payload,
        );
        check(
            SetEntityData {
                entity_id: 77,
                metadata: hex("000020090341a0000010007f"),
            },
            "4d000020090341a0000010007fff",
            SetEntityData::encode_payload,
            SetEntityData::decode_payload,
        );
    }

    #[test]
    fn update_attributes_matches_the_recorded_values() {
        check(
            UpdateAttributes {
                entity_id: 77,
                attributes: vec![
                    AttributeEntry {
                        key: 8,
                        base: 4.5,
                        modifiers: Vec::new(),
                    },
                    AttributeEntry {
                        key: 26,
                        // The trace carries 0.1 as an f32 widened to f64.
                        base: f64::from(0.1_f32),
                        modifiers: Vec::new(),
                    },
                    AttributeEntry {
                        key: 13,
                        base: 3.0,
                        modifiers: Vec::new(),
                    },
                ],
            },
            "4d03084012000000000000001a3fb99999a0000000000d400800000000000000",
            UpdateAttributes::encode_payload,
            UpdateAttributes::decode_payload,
        );
    }

    #[test]
    fn entity_position_sync_matches_the_recorded_player_packet() {
        check(
            EntityPositionSync {
                entity_id: 77,
                teleport_id: 1,
                flags: 1,
                x: 96.7,
                y: 145.0,
                z: -31.5,
                rotation: 2,
                yaw: 45.0,
                pitch: -20.0,
                tail: true,
            },
            "4d010140582ccccccccccd4062200000000000c03f8000000000000242340000c1a0000001",
            EntityPositionSync::encode_payload,
            EntityPositionSync::decode_payload,
        );
    }
}
