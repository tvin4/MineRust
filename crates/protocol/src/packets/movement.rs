//! Clientbound entity movement packets of the M4.5 visibility stream.
//!
//! Every layout is taken from the two-bot vanilla trace (scenarios С1–С4,
//! `docs/ARCH-M4.md` §6) and each codec's test reproduces a recorded payload
//! byte for byte. Header bytes whose meaning the trace does not establish are
//! carried explicitly and marked «не установлено».
//!
//! SOURCE: two-bot vanilla trace of 26.3 (byte layout observed), not wiki.

use super::packet_impl::play_packet;
use crate::codec::{CodecError, PacketReader, PacketWriter};

/// Appends a big-endian `i16`.
fn write_i16(writer: &mut PacketWriter, value: i16) {
    writer.write_u16(value as u16);
}

/// Reads a big-endian `i16`.
fn read_i16(reader: &mut PacketReader<'_>) -> Result<i16, CodecError> {
    Ok(reader.read_u16()? as i16)
}

/// Two-byte header of `move_entity_pos` and `move_entity_pos_rot`.
///
/// `flags` is 0x00 on the first packet after spawn, 0x02 when the delta is
/// zero and 0x03 when the position changed; `kind` is 0x02 for the traced
/// player and 0x01 for mobs, and is absent from the first packet. The meaning
/// of both bytes is «не установлено»; the codec only reproduces the shapes.
// SOURCE: two-bot vanilla trace 26.3 (C1–C4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveHeader {
    /// First header byte.
    pub flags: u8,
    /// Second header byte, absent on the first packet after spawn.
    pub kind: Option<u8>,
}

impl MoveHeader {
    fn encode(&self, writer: &mut PacketWriter) {
        writer.write_u8(self.flags);
        if let Some(kind) = self.kind {
            writer.write_u8(kind);
        }
    }

    /// Decodes the header; `tail` is the number of bytes after it (six delta
    /// bytes, plus two angles for the `_rot` variant).
    fn decode(reader: &mut PacketReader<'_>, tail: usize) -> Result<Self, CodecError> {
        let flags = reader.read_u8()?;
        let kind = match reader.remaining() {
            n if n == tail => None,
            n if n == tail + 1 => Some(reader.read_u8()?),
            remaining => return Err(CodecError::TrailingBytes { remaining }),
        };
        Ok(Self { flags, kind })
    }
}

/// Move Entity Pos: a relative position change without a rotation change.
///
/// Observed payload: `entity id`, [`MoveHeader`], three `i16` deltas in
/// 1/4096 of a block. No `on_ground` byte was recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveEntityPos {
    /// Entity id.
    pub entity_id: i32,
    /// Header bytes (see [`MoveHeader`]).
    pub header: MoveHeader,
    /// X delta in 1/4096 blocks.
    pub dx: i16,
    /// Y delta in 1/4096 blocks.
    pub dy: i16,
    /// Z delta in 1/4096 blocks.
    pub dz: i16,
}

impl MoveEntityPos {
    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.entity_id);
        self.header.encode(&mut writer);
        write_i16(&mut writer, self.dx);
        write_i16(&mut writer, self.dy);
        write_i16(&mut writer, self.dz);
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            entity_id: reader.read_varint()?,
            header: MoveHeader::decode(reader, 6)?,
            dx: read_i16(reader)?,
            dy: read_i16(reader)?,
            dz: read_i16(reader)?,
        })
    }
}

play_packet!(MoveEntityPos, 0x36, "minecraft:move_entity_pos");

/// Move Entity Pos Rot: a relative position change with a body rotation.
///
/// Observed payload (len 11): `entity id`, [`MoveHeader`], three `i16`
/// deltas and two angle bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveEntityPosRot {
    /// Entity id.
    pub entity_id: i32,
    /// Header bytes (see [`MoveHeader`]).
    pub header: MoveHeader,
    /// X delta in 1/4096 blocks.
    pub dx: i16,
    /// Y delta in 1/4096 blocks.
    pub dy: i16,
    /// Z delta in 1/4096 blocks.
    pub dz: i16,
    /// Yaw as a byte angle (`round(deg * 256 / 360)`).
    pub yaw: u8,
    /// Pitch as a signed byte angle.
    pub pitch: i8,
}

impl MoveEntityPosRot {
    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.entity_id);
        self.header.encode(&mut writer);
        write_i16(&mut writer, self.dx);
        write_i16(&mut writer, self.dy);
        write_i16(&mut writer, self.dz);
        writer.write_u8(self.yaw);
        writer.write_i8(self.pitch);
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            entity_id: reader.read_varint()?,
            header: MoveHeader::decode(reader, 8)?,
            dx: read_i16(reader)?,
            dy: read_i16(reader)?,
            dz: read_i16(reader)?,
            yaw: reader.read_u8()?,
            pitch: reader.read_i8()?,
        })
    }
}

play_packet!(MoveEntityPosRot, 0x37, "minecraft:move_entity_pos_rot");

/// Move Entity Rot: a body rotation without a position change.
///
/// Observed payload (len 4): `entity id`, one header byte, yaw and pitch.
/// The header is 0x00 on the first packet and 0x01 afterwards; its meaning
/// is «не установлено».
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveEntityRot {
    /// Entity id.
    pub entity_id: i32,
    /// Header byte; 0x00 first, 0x01 later in the trace.
    pub flags: u8,
    /// Yaw as a byte angle (`round(deg * 256 / 360)`).
    pub yaw: u8,
    /// Pitch as a signed byte angle.
    pub pitch: i8,
}

impl MoveEntityRot {
    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.entity_id);
        writer.write_u8(self.flags);
        writer.write_u8(self.yaw);
        writer.write_i8(self.pitch);
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            entity_id: reader.read_varint()?,
            flags: reader.read_u8()?,
            yaw: reader.read_u8()?,
            pitch: reader.read_i8()?,
        })
    }
}

play_packet!(MoveEntityRot, 0x39, "minecraft:move_entity_rot");

/// Rotate Head: sets the head yaw of an entity.
///
/// Observed payload (len 2): `entity id` and one head-yaw byte angle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RotateHead {
    /// Entity id.
    pub entity_id: i32,
    /// Head yaw as a byte angle (`round(deg * 256 / 360)`).
    pub head_yaw: u8,
}

impl RotateHead {
    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.entity_id);
        writer.write_u8(self.head_yaw);
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            entity_id: reader.read_varint()?,
            head_yaw: reader.read_u8()?,
        })
    }
}

play_packet!(RotateHead, 0x55, "minecraft:rotate_head");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packets::test_support::check;

    #[test]
    fn move_entity_pos_matches_both_recorded_shapes() {
        // First packet after spawn: one header byte.
        check(
            MoveEntityPos {
                entity_id: 77,
                header: MoveHeader {
                    flags: 0x00,
                    kind: None,
                },
                dx: 0,
                dy: 0,
                dz: 0,
            },
            "4d00000000000000",
            MoveEntityPos::encode_payload,
            MoveEntityPos::decode_payload,
        );
        // Walking packet: two header bytes and a 0.2-block delta.
        check(
            MoveEntityPos {
                entity_id: 77,
                header: MoveHeader {
                    flags: 0x03,
                    kind: Some(0x02),
                },
                dx: 819,
                dy: 0,
                dz: 0,
            },
            "4d0302033300000000",
            MoveEntityPos::encode_payload,
            MoveEntityPos::decode_payload,
        );
    }

    #[test]
    fn move_entity_pos_rot_matches_the_recorded_packet() {
        check(
            MoveEntityPosRot {
                entity_id: 77,
                header: MoveHeader {
                    flags: 0x03,
                    kind: Some(0x02),
                },
                dx: 819,
                dy: 0,
                dz: 0,
                yaw: 0xA0,
                pitch: 0x15,
            },
            "4d0302033300000000a015",
            MoveEntityPosRot::encode_payload,
            MoveEntityPosRot::decode_payload,
        );
    }

    #[test]
    fn move_entity_rot_matches_the_recorded_rotation_packets() {
        check(
            MoveEntityRot {
                entity_id: 118,
                flags: 0x00,
                yaw: 0x86,
                pitch: -17,
            },
            "760086ef",
            MoveEntityRot::encode_payload,
            MoveEntityRot::decode_payload,
        );
        check(
            MoveEntityRot {
                entity_id: 118,
                flags: 0x01,
                yaw: 0xA0,
                pitch: 3,
            },
            "7601a003",
            MoveEntityRot::encode_payload,
            MoveEntityRot::decode_payload,
        );
    }

    #[test]
    fn rotate_head_matches_the_recorded_angle() {
        check(
            RotateHead {
                entity_id: 77,
                head_yaw: 0xA0,
            },
            "4da0",
            RotateHead::encode_payload,
            RotateHead::decode_payload,
        );
    }
}
