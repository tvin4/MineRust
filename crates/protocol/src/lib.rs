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
mod packets;
mod status;

pub use codec::{
    decode_varint, encode_varint, CodecError, PacketReader, PacketWriter, MAX_STRING_CHARS,
    MAX_VARINT_BYTES,
};
pub use frame::{
    check_frame_len, decode_frame, decode_frame_len, encode_frame, split_frame, MAX_FRAME_LEN,
};
pub use nbt::{NbtError, NbtTag, MAX_NBT_DEPTH};
pub use packets::{Handshake, NextState, PingRequest, PongResponse, StatusRequest, StatusResponse};
pub use status::StatusInfo;
