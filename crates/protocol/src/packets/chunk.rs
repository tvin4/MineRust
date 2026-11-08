//! Clientbound chunk packets of the play state — the chunk payload itself
//! and the two batch markers around a burst of chunks — plus the
//! serverbound reply that sets the pace of the following batches.
//!
//! The record layouts behind the payload are in [`crate::packets::chunk_payload`]
//! (data from the vanilla join recording, not from a wiki). The batch
//! framing was read from the same recording: one `chunk_batch_start` with
//! an empty payload, nine `level_chunk_with_light` and one
//! `chunk_batch_finished` whose single VarInt equals the number of chunks
//! in the batch (9 in every run of the parity world).
//!
//! SOURCE: parity trace of vanilla 26.3 — ids come from
//! `reports/packets.json`, field layouts from the recorded payloads; the
//! meaning of the fields never exceeds what the bytes demonstrate.

use crate::codec::{CodecError, PacketReader, PacketWriter};
use crate::frame::encode_frame;
use crate::packet_id::{PacketKey, ProtocolState};
use crate::packets::chunk_payload::{
    read_block_entities, read_heightmaps, read_light, read_sections, write_block_entities,
    write_heightmaps, write_light, write_sections, BlockEntity, ChunkSection, HeightmapRecord,
    LightData,
};

/// One chunk with the light of its sections: the payload carried by
/// `minecraft:level_chunk_with_light`.
///
/// Decoding follows the recorded layout in order (coordinates, heightmaps,
/// section blob, block entities, light) and validates every structural rule
/// the recording follows; encoding writes the same order back byte for byte.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelChunkWithLight {
    /// Chunk x coordinate.
    pub chunk_x: i32,
    /// Chunk z coordinate.
    pub chunk_z: i32,
    /// Heightmaps as the wire sends them (ids 1, 4, 5 in the recording).
    pub heightmaps: Vec<HeightmapRecord>,
    /// Sections from the bottom of the world up, one per vertical slot.
    pub sections: Vec<ChunkSection>,
    /// Block entities of this chunk.
    pub block_entities: Vec<BlockEntity>,
    /// The light block: masks, then both array groups.
    pub light: LightData,
}

impl LevelChunkWithLight {
    /// Packet id of Chunk Data and Update Light (play, clientbound).
    pub const ID: u32 = 0x2E;

    /// Key that resolves the Chunk Data id at runtime.
    pub const KEY: PacketKey = PacketKey::clientbound(
        ProtocolState::Play,
        "minecraft:level_chunk_with_light",
        Self::ID,
    );

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        let chunk_x = reader.read_i32()?;
        let chunk_z = reader.read_i32()?;
        let heightmaps = read_heightmaps(reader)?;
        let sections = read_sections(reader)?;
        let block_entities = read_block_entities(reader)?;
        let light = read_light(reader)?;
        Ok(Self {
            chunk_x,
            chunk_z,
            heightmaps,
            sections,
            block_entities,
            light,
        })
    }

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_i32(self.chunk_x);
        writer.write_i32(self.chunk_z);
        write_heightmaps(&mut writer, &self.heightmaps)?;
        write_sections(&mut writer, &self.sections)?;
        write_block_entities(&mut writer, &self.block_entities)?;
        write_light(&mut writer, &self.light)?;
        Ok(writer.into_bytes())
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload()?)
    }
}

/// Clientbound Chunk Batch Start: opens a burst of chunks; the payload is
/// empty in every recording, so there is nothing to decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkBatchStart;

impl ChunkBatchStart {
    /// Packet id of Chunk Batch Start (play, clientbound).
    pub const ID: u32 = 0x0C;

    /// Key that resolves the Chunk Batch Start id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:chunk_batch_start", Self::ID);

    /// Encodes the payload (empty, as recorded).
    pub fn encode_payload(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Decodes the payload; an empty payload is the only valid one.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        reader.ensure_empty()?;
        Ok(Self)
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Clientbound Chunk Batch Finished: closes a burst of chunks.
///
/// The wire carries a single VarInt: in all four recordings it equals the
/// number of chunk packets between start and finished (9), so it is read
/// as "chunks in this batch"; no source demonstrates any other meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkBatchFinished {
    /// Number of chunk packets in the batch, as recorded.
    pub chunk_count: i32,
}

impl ChunkBatchFinished {
    /// Packet id of Chunk Batch Finished (play, clientbound).
    pub const ID: u32 = 0x0B;

    /// Key that resolves the Chunk Batch Finished id at runtime.
    pub const KEY: PacketKey = PacketKey::clientbound(
        ProtocolState::Play,
        "minecraft:chunk_batch_finished",
        Self::ID,
    );

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.chunk_count);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        let chunk_count = reader.read_varint()?;
        if chunk_count < 0 {
            return Err(CodecError::InvalidChunk("negative batch chunk count"));
        }
        Ok(Self { chunk_count })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Serverbound Chunk Batch Received: answered to every
/// `chunk_batch_finished`, it tells the server how fast the client wants
/// the following batches to arrive.
// SOURCE: wiki, unverified — the composition (a single Float called
// "Chunks Per Tick") is the documented one; `data/reports/packets.json`
// only gives the packet's id (11). Whether a vanilla server accepts this
// payload is checked by the parity run of step 3.3d: the bot sends it
// after `chunk_batch_finished`, and a stream that keeps flowing past the
// reply proves the composition (result in `docs/SOURCES.md`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChunkBatchReceived {
    /// Chunks per tick the client asks the server to send.
    pub chunks_per_tick: f32,
}

impl ChunkBatchReceived {
    /// Packet id of Chunk Batch Received (play, serverbound).
    ///
    /// The literal is the report's `protocol_id` — a verified value of
    /// `reports/packets.json`, not a documentation guess.
    pub const ID: u32 = 11;

    /// Key that resolves the Chunk Batch Received id at runtime.
    pub const KEY: PacketKey = PacketKey::serverbound(
        ProtocolState::Play,
        "minecraft:chunk_batch_received",
        Self::ID,
    );

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_f32(self.chunks_per_tick);
        writer.into_bytes()
    }

    /// Decodes the payload; an empty payload is the only valid one.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            chunks_per_tick: reader.read_f32()?,
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
    use crate::packets::chunk_payload::{PaletteContainer, LIGHT_ARRAY_BYTES};

    /// A minimal but well-formed payload: one heightmap, two sections (a
    /// single-valued one and a packed one), a block entity with the empty
    /// NBT record and a light block sized to both masks.
    fn sample_chunk() -> LevelChunkWithLight {
        LevelChunkWithLight {
            chunk_x: 6,
            chunk_z: -2,
            heightmaps: vec![HeightmapRecord {
                id: 4,
                words: vec![1_234_567, -77],
            }],
            sections: vec![
                ChunkSection {
                    non_air: 0,
                    fluid: 0,
                    blocks: PaletteContainer::Single(0),
                    biomes: PaletteContainer::Single(1),
                },
                ChunkSection {
                    non_air: 512,
                    fluid: 0,
                    blocks: PaletteContainer::Packed {
                        bits: 4,
                        palette: vec![0, 1, 30],
                        data: vec![0; 2048],
                    },
                    biomes: PaletteContainer::Packed {
                        bits: 2,
                        palette: vec![0, 1, 2, 3],
                        data: vec![0x01; 16],
                    },
                },
            ],
            block_entities: vec![BlockEntity {
                local_x: 6,
                local_z: 11,
                y: -20,
                type_id: 34,
                nbt: vec![0x00],
            }],
            light: LightData {
                sky_mask: vec![0x01],
                block_mask: vec![0x01],
                empty_sky_mask: vec![0x00],
                empty_block_mask: vec![0x00],
                sky_arrays: vec![vec![0x5A; LIGHT_ARRAY_BYTES]],
                block_arrays: vec![vec![0xA5; LIGHT_ARRAY_BYTES]],
            },
        }
    }

    #[test]
    fn a_chunk_payload_roundtrips() {
        let packet = sample_chunk();
        let payload = packet.encode_payload().expect("encodable");
        let mut reader = PacketReader::new(&payload);
        let decoded = LevelChunkWithLight::decode_payload(&mut reader).expect("decodable");
        reader.ensure_empty().expect("no trailing bytes");
        assert_eq!(decoded, packet);
        assert_eq!(decoded.chunk_x, 6);
        assert_eq!(decoded.chunk_z, -2);
        assert_eq!(decoded.sections.len(), 2, "sections per chunk");
        assert_eq!(decoded.heightmaps[0].id, 4);
        assert_eq!(decoded.block_entities[0].nbt, vec![0x00]);
    }

    #[test]
    fn a_payload_with_trailing_bytes_is_not_silently_accepted() {
        let mut payload = sample_chunk().encode_payload().expect("encodable");
        payload.push(0x00);
        let mut reader = PacketReader::new(&payload);
        LevelChunkWithLight::decode_payload(&mut reader).expect("decodable");
        let err = reader.ensure_empty().expect_err("trailing byte");
        assert!(
            matches!(err, CodecError::TrailingBytes { remaining: 1 }),
            "{err}"
        );
    }

    #[test]
    fn batch_start_and_finish_frame_a_burst() {
        let start = ChunkBatchStart;
        let decoded =
            ChunkBatchStart::decode_payload(&mut PacketReader::new(&[])).expect("decodable");
        assert_eq!(decoded, start);
        let start_frame = start.to_frame().expect("frameable");
        assert_eq!(
            start_frame.last().copied(),
            Some(ChunkBatchStart::ID as u8 & 0x7F)
        );

        let finished = ChunkBatchFinished { chunk_count: 9 };
        let payload = finished.encode_payload();
        // The recorded vanilla payload of the same packet.
        assert_eq!(payload, vec![0x09]);
        let mut reader = PacketReader::new(&payload);
        let decoded = ChunkBatchFinished::decode_payload(&mut reader).expect("decodable");
        reader.ensure_empty().expect("no trailing bytes");
        assert_eq!(decoded, finished);
    }

    /// The pace reply of step 3.3d is exactly one float: four bytes of
    /// payload, the id from the report (11), and nothing else.
    #[test]
    fn batch_received_carries_one_float() {
        let packet = ChunkBatchReceived {
            chunks_per_tick: 10.5,
        };
        let payload = packet.encode_payload();
        assert_eq!(payload.len(), 4, "a single float on the wire");
        let mut reader = PacketReader::new(&payload);
        let decoded =
            ChunkBatchReceived::decode_payload(&mut reader).expect("decodable only this far");
        reader.ensure_empty().expect("no trailing bytes");
        assert_eq!(decoded.chunks_per_tick, 10.5, "round-tripped unchanged");
        assert_eq!(ChunkBatchReceived::ID, 11, "the report's protocol_id");
        assert_eq!(ChunkBatchReceived::KEY.default_id, 11, "same as the id");
        assert_eq!(
            ChunkBatchReceived::KEY.name,
            "minecraft:chunk_batch_received"
        );
    }

    #[test]
    fn a_negative_batch_count_is_rejected() {
        let bytes = [0xFF, 0xFF, 0xFF, 0xFF, 0x0F];
        let mut reader = PacketReader::new(&bytes);
        let err = ChunkBatchFinished::decode_payload(&mut reader).expect_err("negative");
        assert!(matches!(err, CodecError::InvalidChunk(_)), "{err}");
    }
}
