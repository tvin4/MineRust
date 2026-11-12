//! Clientbound entity packets of the M4.5 visibility stream: add, remove and
//! tab-list removal.
//!
//! Every layout is taken from the two-bot vanilla trace (scenarios С1–С4,
//! `docs/ARCH-M4.md` §6) and each codec's test reproduces a recorded payload
//! byte for byte. Bytes whose meaning the trace does not establish are
//! carried explicitly and marked «не установлено» — nothing is guessed.
//!
//! SOURCE: two-bot vanilla trace of 26.3 (byte layout observed), not wiki.

use super::packet_impl::play_packet;
use super::read_count;
use crate::codec::{CodecError, PacketReader, PacketWriter, Uuid};

/// Add Entity: makes an entity known to the client.
///
/// Observed payload (len 48): `entity id`, UUID, entity type, three `f64`
/// coordinates, three angle bytes, a `VarInt` data value and one trailing
/// byte. The last five bytes are all zero in every scenario, so neither the
/// order of the angles nor the meaning of the trailing byte is established.
// SOURCE: two-bot vanilla trace 26.3 (C1–C4), one observed value per field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AddEntity {
    /// Entity id assigned by the server for this connection.
    pub entity_id: i32,
    /// Offline UUID of the player.
    pub uuid: Uuid,
    /// Entity type id (159 for the traced player).
    pub entity_type: i32,
    /// X coordinate.
    pub x: f64,
    /// Y coordinate.
    pub y: f64,
    /// Z coordinate.
    pub z: f64,
    /// First angle byte; order with the other two is «не установлено».
    pub pitch: u8,
    /// Second angle byte; order is «не установлено».
    pub yaw: u8,
    /// Third angle byte; order is «не установлено».
    pub head_yaw: u8,
    /// Metadata-ish `VarInt`; always 0 in the trace, semantics «не установлено».
    pub data: i32,
    /// Fifth tail byte; always 0 in the trace, semantics «не установлено».
    pub tail: u8,
}

impl AddEntity {
    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.entity_id);
        writer.write_uuid(self.uuid);
        writer.write_varint(self.entity_type);
        writer.write_f64(self.x);
        writer.write_f64(self.y);
        writer.write_f64(self.z);
        writer.write_u8(self.pitch);
        writer.write_u8(self.yaw);
        writer.write_u8(self.head_yaw);
        writer.write_varint(self.data);
        writer.write_u8(self.tail);
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            entity_id: reader.read_varint()?,
            uuid: reader.read_uuid()?,
            entity_type: reader.read_varint()?,
            x: reader.read_f64()?,
            y: reader.read_f64()?,
            z: reader.read_f64()?,
            pitch: reader.read_u8()?,
            yaw: reader.read_u8()?,
            head_yaw: reader.read_u8()?,
            data: reader.read_varint()?,
            tail: reader.read_u8()?,
        })
    }
}

play_packet!(AddEntity, 0x01, "minecraft:add_entity");

/// Remove Entities: drops entities from the client's world.
///
/// Observed payload (len 2 for one entity): `VarInt` count followed by that
/// many entity ids.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemoveEntities {
    /// Entity ids to remove.
    pub entity_ids: Vec<i32>,
}

impl RemoveEntities {
    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.entity_ids.len() as i32);
        for entity_id in &self.entity_ids {
            writer.write_varint(*entity_id);
        }
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        let count = read_count(reader)?;
        let mut entity_ids = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            entity_ids.push(reader.read_varint()?);
        }
        Ok(Self { entity_ids })
    }
}

play_packet!(RemoveEntities, 0x4E, "minecraft:remove_entities");

/// Player Info Remove: removes players from the tab list.
///
/// Observed payload (len 17): `VarInt` count followed by that many UUIDs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlayerInfoRemove {
    /// UUIDs to remove.
    pub uuids: Vec<Uuid>,
}

impl PlayerInfoRemove {
    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.uuids.len() as i32);
        for uuid in &self.uuids {
            writer.write_uuid(*uuid);
        }
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        let count = read_count(reader)?;
        let mut uuids = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            uuids.push(reader.read_uuid()?);
        }
        Ok(Self { uuids })
    }
}

play_packet!(PlayerInfoRemove, 0x46, "minecraft:player_info_remove");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::test_support::{check, uuid};
    use crate::packets::PlayerInfoEntry;

    #[test]
    fn add_entity_matches_the_recorded_join_packet() {
        check(
            AddEntity {
                entity_id: 77,
                uuid: uuid("b3b03a004714337e810352c60a6c836f"),
                entity_type: 159,
                x: 96.5,
                y: 145.0,
                z: -31.5,
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
                tail: 0,
            },
            "4db3b03a004714337e810352c60a6c836f9f0140582000000000004062200000000000c03f8000000000000000000000",
            AddEntity::encode_payload,
            AddEntity::decode_payload,
        );
    }

    #[test]
    fn remove_entities_matches_the_recorded_leave() {
        check(
            RemoveEntities {
                entity_ids: vec![77],
            },
            "014d",
            RemoveEntities::encode_payload,
            RemoveEntities::decode_payload,
        );
    }

    #[test]
    fn player_info_remove_matches_the_recorded_uuid() {
        check(
            PlayerInfoRemove {
                uuids: vec![uuid("b3b03a004714337e810352c60a6c836f")],
            },
            "01b3b03a004714337e810352c60a6c836f",
            PlayerInfoRemove::encode_payload,
            PlayerInfoRemove::decode_payload,
        );
    }

    #[test]
    fn player_info_update_matches_the_recorded_other_player() {
        use crate::packets::PlayerInfoUpdate;

        // The join stream of another player, exactly as recorded for B.
        let packet = PlayerInfoUpdate {
            players: vec![PlayerInfoEntry {
                uuid: uuid("b3b03a004714337e810352c60a6c836f"),
                name: "TraceBotB".to_owned(),
                properties: Vec::new(),
                game_mode: 3,
                listed: true,
                latency: 0,
            }],
        };
        let expected = crate::packets::test_support::hex(
            "ff01b3b03a004714337e810352c60a6c836f095472616365426f74420000030100000001",
        );
        assert_eq!(packet.encode_payload().expect("encodable"), expected);
        let mut reader = PacketReader::new(&expected);
        let decoded = PlayerInfoUpdate::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }
}
