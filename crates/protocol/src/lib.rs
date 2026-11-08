//! Packet definitions and codecs for Minecraft: Java Edition 26.3.
//!
//! The wire format follows the public protocol documentation
//! (<https://minecraft.wiki/wiki/Java_Edition_protocol>): every packet is
//! framed as `Length (VarInt) + Packet ID (VarInt) + Data`, and every value
//! except VarInt/VarLong is big-endian.
//!
//! The protocol version number and version name of a particular game release
//! are **not** stored in this repository. They are supplied at runtime by the
//! server configuration (see the `server` crate), which is filled in from the
//! user's own copy of the game.

mod codec;
mod frame;
mod nbt;
mod packet_id;
mod packets;
mod status;

pub use packet_id::{Direction, PacketKey, ProtocolState};

pub use codec::{
    decode_varint, encode_varint, CodecError, PacketReader, PacketWriter, Position, Uuid,
    MAX_STRING_CHARS, MAX_VARINT_BYTES,
};
pub use frame::{
    check_frame_len, decode_frame, decode_frame_len, encode_frame, split_frame, MAX_FRAME_LEN,
};
pub use nbt::{NbtError, NbtTag, MAX_NBT_DEPTH};
pub use packets::{
    hashed_seed, offline_uuid, set_bits, used_packet_keys, BlockEntity, ChunkBatchFinished,
    ChunkBatchReceived, ChunkBatchStart, ChunkSection, ClientInformation, ConfirmTeleportation,
    DeathLocation, DisconnectLogin, FinishAck, FinishConfiguration, GameEvent, GameProfile,
    Handshake, HeightmapRecord, KeepAlive, LevelChunkWithLight, LightData, LoginAcknowledged,
    LoginPlay, LoginStart, LoginSuccess, NextState, PaletteContainer, PingRequest, PlayerInfoEntry,
    PlayerInfoUpdate, PongResponse, ProfileProperty, RegistryData, RegistryEntry, RegistryTag,
    SetCenterChunk, SetCompression, SetDefaultSpawnPosition, StatusRequest, StatusResponse,
    SynchronizePlayerPosition, TagRegistry, UpdateTags, BIOME_CELLS, BLOCK_CELLS,
    CONFIGURATION_CLIENTBOUND_PLUGIN_MESSAGE, CONFIGURATION_PLUGIN_MESSAGE, LIGHT_ARRAY_BYTES,
    MOVE_PLAYER_POS, MOVE_PLAYER_POS_ROT, MOVE_PLAYER_ROT, MOVE_PLAYER_STATUS_ONLY,
    PLAY_PLUGIN_MESSAGE, SELECT_KNOWN_PACKS, SELECT_KNOWN_PACKS_SERVERBOUND,
    UPDATE_ENABLED_FEATURES,
};
pub use status::StatusInfo;
