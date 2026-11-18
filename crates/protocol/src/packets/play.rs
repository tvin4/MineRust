//! Play state packets: the join sequence plus the keep-alive and teleport
//! handshake that keeps a connection alive.
//!
//! The join sequence sent by this server (see the `net` crate) is Login,
//! Set Default Spawn Position, Player Info Update, Set Center Chunk, Game
//! Event and Synchronize Player Position. Field layouts follow the public
//! protocol documentation (<https://minecraft.wiki/wiki/Java_Edition_protocol>).
//!
//! SOURCE: wiki, unverified — the sequence and the fields come from the
//! documented protocol; no vanilla server has compared them yet.

use crate::codec::{CodecError, PacketReader, PacketWriter, Position, Uuid, MAX_STRING_CHARS};
use crate::frame::encode_frame;
use crate::nbt::NbtTag;
use crate::packet_id::{PacketKey, ProtocolState};
use crate::packets::login::{ProfileProperty, MAX_PROPERTY_NAME_CHARS, MAX_SIGNATURE_CHARS};

use std::collections::BTreeMap;

/// Maximum locale length in UTF-16 code units (protocol String (16)), same
/// as in the configuration state.
const MAX_LOCALE_CHARS: usize = 16;

/// Key of the serverbound Plugin Message packet in the play state. The
/// channel name is logged and the payload is ignored (M2 has no gameplay).
pub const PLAY_PLUGIN_MESSAGE: PacketKey =
    PacketKey::serverbound(ProtocolState::Play, "minecraft:custom_payload", 0x16);

/// Key of the serverbound position packet in the play state (x, y, z).
pub const MOVE_PLAYER_POS: PacketKey =
    PacketKey::serverbound(ProtocolState::Play, "minecraft:move_player_pos", 0x1E);

/// Key of the serverbound position + rotation packet in the play state.
pub const MOVE_PLAYER_POS_ROT: PacketKey =
    PacketKey::serverbound(ProtocolState::Play, "minecraft:move_player_pos_rot", 0x1F);

/// Key of the serverbound rotation-only packet in the play state.
pub const MOVE_PLAYER_ROT: PacketKey =
    PacketKey::serverbound(ProtocolState::Play, "minecraft:move_player_rot", 0x20);

/// Key of the serverbound "still standing" packet in the play state.
pub const MOVE_PLAYER_STATUS_ONLY: PacketKey = PacketKey::serverbound(
    ProtocolState::Play,
    "minecraft:move_player_status_only",
    0x21,
);

/// Clientbound Login (play): the first packet of the play state; it defines
/// the dimension, game mode and rendering parameters the client starts with.
#[derive(Debug, Clone, PartialEq)]
pub struct LoginPlay {
    /// Entity id of the player on this connection.
    pub entity_id: i32,
    /// Whether the connection is a hardcore mode session.
    pub hardcore: bool,
    /// Identifiers of every dimension on the server.
    pub dimension_names: Vec<String>,
    /// Maximum player count (informational only).
    pub max_players: i32,
    /// Render distance the server allows (2-32).
    pub view_distance: i32,
    /// Server simulation distance in chunks.
    pub simulation_distance: i32,
    /// Whether the debug (F3) screen is reduced.
    pub reduced_debug_info: bool,
    /// Whether the respawn screen is shown on death.
    pub enable_respawn_screen: bool,
    /// Whether crafting requires unlocked recipes.
    pub do_limited_crafting: bool,
    /// Numeric id of the dimension type in the `minecraft:dimension_type`
    /// registry (an entry of the Registry Data packets).
    pub dimension_type: i32,
    /// Identifier of the dimension the player spawns into.
    pub dimension_name: String,
    /// First 8 bytes of the SHA-256 hash of the world seed.
    pub hashed_seed: i64,
    /// Game mode: 0 survival, 1 creative, 2 adventure, 3 spectator.
    pub game_mode: i32,
    /// Previous game mode: 0 undefined, otherwise the game mode id + 1.
    pub previous_game_mode: i32,
    /// Whether the world is a debug mode world.
    pub is_debug: bool,
    /// Whether the world is a superflat world.
    pub is_flat: bool,
    /// Where the player last died, when known.
    pub death_location: Option<DeathLocation>,
    /// Ticks until the player may use the last portal again.
    pub portal_cooldown: i32,
    /// Sea level of the dimension.
    pub sea_level: i32,
    /// Whether the server is in online mode.
    pub online_mode: bool,
    /// Whether secure chat is enforced.
    pub enforces_secure_chat: bool,
}

/// The place a player died at, as carried by [`LoginPlay`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeathLocation {
    /// Dimension the death happened in.
    pub dimension: String,
    /// Block position of the death.
    pub position: Position,
}

impl LoginPlay {
    /// Packet id of Login (play state, clientbound).
    pub const ID: u32 = 0x32;

    /// Key that resolves the Login (play) id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:login", Self::ID);

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_i32(self.entity_id);
        writer.write_bool(self.hardcore);
        writer.write_varint(self.dimension_names.len() as i32);
        for name in &self.dimension_names {
            writer.write_string(name, MAX_STRING_CHARS)?;
        }
        writer.write_varint(self.max_players);
        writer.write_varint(self.view_distance);
        writer.write_varint(self.simulation_distance);
        writer.write_bool(self.reduced_debug_info);
        writer.write_bool(self.enable_respawn_screen);
        writer.write_bool(self.do_limited_crafting);
        writer.write_varint(self.dimension_type);
        writer.write_string(&self.dimension_name, MAX_STRING_CHARS)?;
        writer.write_i64(self.hashed_seed);
        writer.write_varint(self.game_mode);
        writer.write_varint(self.previous_game_mode);
        writer.write_bool(self.is_debug);
        writer.write_bool(self.is_flat);
        match &self.death_location {
            Some(death) => {
                writer.write_bool(true);
                writer.write_string(&death.dimension, MAX_STRING_CHARS)?;
                writer.write_position(death.position);
            }
            None => writer.write_bool(false),
        }
        writer.write_varint(self.portal_cooldown);
        writer.write_varint(self.sea_level);
        writer.write_bool(self.online_mode);
        writer.write_bool(self.enforces_secure_chat);
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        let entity_id = reader.read_i32()?;
        let hardcore = reader.read_bool()?;
        let count = read_count(reader)?;
        let mut dimension_names = Vec::with_capacity(count.min(16));
        for _ in 0..count {
            dimension_names.push(reader.read_string(MAX_STRING_CHARS)?);
        }
        let max_players = reader.read_varint()?;
        let view_distance = reader.read_varint()?;
        let simulation_distance = reader.read_varint()?;
        let reduced_debug_info = reader.read_bool()?;
        let enable_respawn_screen = reader.read_bool()?;
        let do_limited_crafting = reader.read_bool()?;
        let dimension_type = reader.read_varint()?;
        let dimension_name = reader.read_string(MAX_STRING_CHARS)?;
        let hashed_seed = reader.read_i64()?;
        let game_mode = reader.read_varint()?;
        let previous_game_mode = reader.read_varint()?;
        let is_debug = reader.read_bool()?;
        let is_flat = reader.read_bool()?;
        let death_location = if reader.read_bool()? {
            Some(DeathLocation {
                dimension: reader.read_string(MAX_STRING_CHARS)?,
                position: reader.read_position()?,
            })
        } else {
            None
        };
        let portal_cooldown = reader.read_varint()?;
        let sea_level = reader.read_varint()?;
        let online_mode = reader.read_bool()?;
        let enforces_secure_chat = reader.read_bool()?;
        Ok(Self {
            entity_id,
            hardcore,
            dimension_names,
            max_players,
            view_distance,
            simulation_distance,
            reduced_debug_info,
            enable_respawn_screen,
            do_limited_crafting,
            dimension_type,
            dimension_name,
            hashed_seed,
            game_mode,
            previous_game_mode,
            is_debug,
            is_flat,
            death_location,
            portal_cooldown,
            sea_level,
            online_mode,
            enforces_secure_chat,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload()?)
    }
}

/// Clientbound Disconnect (play): the server ends the session and the client
/// shows the reason. The reason is a text component in the network NBT form
/// (a nameless root compound), not a JSON string.
///
/// SOURCE: the S15 vanilla trace (`2026-11-17_s15-vanilla-r2`): the duplicate
/// login kicks the old connection with
/// `0a 08 0009 "translate" 0026 "multiplayer.disconnect.duplicate_login" 00`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectPlay {
    /// Translation key of the reason, no arguments.
    pub reason: String,
}

impl DisconnectPlay {
    /// Packet id of Disconnect (play state, clientbound).
    pub const ID: u32 = 0x20;

    /// Key that resolves the Disconnect id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:disconnect", Self::ID);

    /// The reason vanilla sends when the same account logs in again (S15).
    pub const DUPLICATE_LOGIN: &'static str = "multiplayer.disconnect.duplicate_login";

    /// Encodes the payload (packet id excluded): `{"translate": reason}` as a
    /// nameless-root NBT compound.
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut map = BTreeMap::new();
        map.insert("translate".to_owned(), NbtTag::String(self.reason.clone()));
        Ok(NbtTag::Compound(map).to_network_bytes()?)
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload()?)
    }
}

/// Hashes a world seed the way the `hashed_seed` field of [`LoginPlay`]
/// carries it: SHA-256 of the seed's 8 little-endian bytes, the first 8
/// digest bytes read back as a little-endian i64 (written big-endian on the
/// wire like every fixed-size integer).
///
/// SOURCE: parity recording of vanilla 26.3 — verified byte for byte: the
/// parity world has `level-seed=12345` and the vanilla `minecraft:login`
/// payload ends with `0x0413_9124_5135_54E1`, which is exactly
/// `hashed_seed(12345)`.
pub fn hashed_seed(seed: i64) -> i64 {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(seed.to_le_bytes());
    i64::from_le_bytes(digest[..8].try_into().unwrap_or([0; 8]))
}

/// Clientbound Game Event: a one-shot world event. Event 13 ("start waiting
/// for chunks") is required after Login (play) for the client to leave the
/// loading screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameEvent {
    /// Event id; 13 starts waiting for chunks after Login.
    pub event: u8,
    /// Event-specific value (unused by event 13).
    pub value: f32,
}

impl GameEvent {
    /// Packet id of Game Event (play state, clientbound).
    pub const ID: u32 = 0x27;

    /// Key that resolves the Game Event id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:game_event", Self::ID);

    /// The event that must follow every Login (play) so the client closes
    /// its loading screen.
    // SOURCE: wiki, unverified: event numbering is documented, not read
    // from game data.
    pub const START_WAITING_FOR_CHUNKS: u8 = 13;

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_u8(self.event);
        writer.write_f32(self.value);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            event: reader.read_u8()?,
            value: reader.read_f32()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Clientbound Set Center Chunk: the center of the client's chunk loading
/// area, in chunk coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetCenterChunk {
    /// Chunk X of the center.
    pub chunk_x: i32,
    /// Chunk Z of the center.
    pub chunk_z: i32,
}

impl SetCenterChunk {
    /// Packet id of Set Center Chunk (play state, clientbound).
    pub const ID: u32 = 0x60;

    /// Key that resolves the Set Center Chunk id at runtime.
    pub const KEY: PacketKey = PacketKey::clientbound(
        ProtocolState::Play,
        "minecraft:set_chunk_cache_center",
        Self::ID,
    );

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.chunk_x);
        writer.write_varint(self.chunk_z);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            chunk_x: reader.read_varint()?,
            chunk_z: reader.read_varint()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Clientbound Forget Level Chunk: tells the client to drop the chunk at
/// `(chunk_x, chunk_z)` from its cache, one packet per chunk.
///
/// The payload is one big-endian `i64` (8 bytes): the packed `ChunkPos`, the
/// **z** in the high word and the **x** in the low word (`pos.toLong() =
/// (z << 32) | x`), so on the wire z comes first. Not two independent
/// `(x, z)` words and not VarInts. The vanilla 26.3 walking traces prove it
/// (`window-p4/p5.jsonl`, M4.4 B): reading the first word as x makes the
/// forgotten column miss `window(old) − window(new)`, reading it as z
/// matches it byte for byte. SOURCE: parity trace; id from
/// `reports/packets.json` (protocol id 38).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForgetLevelChunk {
    /// Chunk X to drop (the low word of the packed position).
    pub chunk_x: i32,
    /// Chunk Z to drop (the high word of the packed position).
    pub chunk_z: i32,
}

impl ForgetLevelChunk {
    /// Packet id of Forget Level Chunk (play state, clientbound).
    pub const ID: u32 = 0x26;

    /// Key that resolves the Forget Level Chunk id at runtime.
    pub const KEY: PacketKey = PacketKey::clientbound(
        ProtocolState::Play,
        "minecraft:forget_level_chunk",
        Self::ID,
    );

    /// Encodes the payload (packet id excluded): the packed `ChunkPos`, z in
    /// the high word and x in the low word.
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_i32(self.chunk_z);
        writer.write_i32(self.chunk_x);
        writer.into_bytes()
    }

    /// Decodes the payload (the packed `ChunkPos`); the packet id must
    /// already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            chunk_z: reader.read_i32()?,
            chunk_x: reader.read_i32()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Clientbound Set Default Spawn Position: where compasses point and where
/// players spawn without a bed.
#[derive(Debug, Clone, PartialEq)]
pub struct SetDefaultSpawnPosition {
    /// Dimension of the spawn point.
    pub dimension: String,
    /// Block position of the spawn point.
    pub position: Position,
    /// Yaw after respawning.
    pub yaw: f32,
    /// Pitch after respawning.
    pub pitch: f32,
}

impl SetDefaultSpawnPosition {
    /// Packet id of Set Default Spawn Position (play state, clientbound).
    pub const ID: u32 = 0x63;

    /// Key that resolves the Set Default Spawn Position id at runtime.
    pub const KEY: PacketKey = PacketKey::clientbound(
        ProtocolState::Play,
        "minecraft:set_default_spawn_position",
        Self::ID,
    );

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_string(&self.dimension, MAX_STRING_CHARS)?;
        writer.write_position(self.position);
        writer.write_f32(self.yaw);
        writer.write_f32(self.pitch);
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            dimension: reader.read_string(MAX_STRING_CHARS)?,
            position: reader.read_position()?,
            yaw: reader.read_f32()?,
            pitch: reader.read_f32()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload()?)
    }
}

/// One row of the tab list as sent by [`PlayerInfoUpdate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfoEntry {
    /// UUID of the player.
    pub uuid: Uuid,
    /// Username.
    pub name: String,
    /// Profile properties (usually empty in offline mode).
    pub properties: Vec<ProfileProperty>,
    /// Game mode shown in the tab list: 0 survival, 1 creative, ...
    pub game_mode: i32,
    /// Whether the player appears in the tab list at all.
    pub listed: bool,
    /// Latency in milliseconds; 0 shows five bars.
    pub latency: i32,
}

/// Clientbound Player Info Update: adds players to the tab list.
///
/// The mask and per-player layout are the vanilla join recording's; see
/// [`PLAYER_INFO_ACTIONS`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlayerInfoUpdate {
    /// Players to add or update.
    pub players: Vec<PlayerInfoEntry>,
}

/// Action mask of [`PlayerInfoUpdate`]: all eight actions, as the vanilla
/// recording sends the joining player (`join-vanilla.json`, packet #56).
///
/// Wire layout, observed byte for byte: `mask (u8)`, `count (VarInt)`,
/// `count` UUIDs, then per player the data of every set action in bit
/// order:
///
/// 1. add player — name + properties;
/// 2. initialize chat — one byte, `0x00` (no session in an offline join);
/// 3. update game mode — VarInt;
/// 4. update listed — bool;
/// 5. update latency — VarInt;
/// 6. update display name — optional, `0x00` (absent);
/// 7. update chat session — one byte, `0x00` (absent);
/// 8. (26.3) a single byte, `0x01` on join.
///
/// The identity/order of the first seven actions and the values above come
/// from the recording; the meaning of the eighth action is not verified.
// SOURCE: parity recording of vanilla 26.3 (byte layout observed); the
// eighth action is unverified and only reproduced as recorded.
const PLAYER_INFO_ACTIONS: u8 = 0xFF;

/// Payload byte of the eighth (unverified) player info action on join, as
/// recorded; see [`PLAYER_INFO_ACTIONS`].
// SOURCE: parity recording of vanilla 26.3, one observed value.
const PLAYER_INFO_LAST_ACTION: u8 = 0x01;

impl PlayerInfoUpdate {
    /// Packet id of Player Info Update (play state, clientbound).
    pub const ID: u32 = 0x47;

    /// Key that resolves the Player Info Update id at runtime.
    pub const KEY: PacketKey = PacketKey::clientbound(
        ProtocolState::Play,
        "minecraft:player_info_update",
        Self::ID,
    );

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_u8(PLAYER_INFO_ACTIONS);
        writer.write_varint(self.players.len() as i32);
        for player in &self.players {
            writer.write_uuid(player.uuid);
        }
        for player in &self.players {
            writer.write_string(&player.name, MAX_LOCALE_CHARS)?;
            writer.write_varint(player.properties.len() as i32);
            for property in &player.properties {
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
            // The seven documented actions plus the eighth byte, in the
            // bit order of the vanilla recording (see PLAYER_INFO_ACTIONS).
            writer.write_bool(false); // initialize chat: no session
            writer.write_varint(player.game_mode);
            writer.write_bool(player.listed);
            writer.write_varint(player.latency);
            writer.write_bool(false); // display name: absent
            writer.write_bool(false); // chat session: absent
            writer.write_u8(PLAYER_INFO_LAST_ACTION); // as recorded
        }
        Ok(writer.into_bytes())
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        let actions = reader.read_u8()?;
        if actions != PLAYER_INFO_ACTIONS {
            return Err(CodecError::TrailingBytes {
                remaining: usize::from(actions),
            });
        }
        let count = read_count(reader)?;
        let mut uuids = Vec::with_capacity(count.min(64));
        for _ in 0..count {
            uuids.push(reader.read_uuid()?);
        }
        let mut players = Vec::with_capacity(uuids.len());
        for uuid in uuids {
            let name = reader.read_string(MAX_LOCALE_CHARS)?;
            let property_count = read_count(reader)?;
            let mut properties = Vec::with_capacity(property_count.min(8));
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
            // Actions 2..=8 of the recording's layout; only the shapes the
            // recording shows (absent optional sections) are modelled.
            if reader.read_bool()? {
                return Err(CodecError::Unsupported {
                    context: "initialized chat session",
                });
            }
            let game_mode = reader.read_varint()?;
            let listed = reader.read_bool()?;
            let latency = reader.read_varint()?;
            if reader.read_bool()? {
                return Err(CodecError::Unsupported {
                    context: "player info display name",
                });
            }
            if reader.read_bool()? {
                return Err(CodecError::Unsupported {
                    context: "player info chat session",
                });
            }
            reader.read_u8()?;
            players.push(PlayerInfoEntry {
                uuid,
                name,
                properties,
                game_mode,
                listed,
                latency,
            });
        }
        Ok(Self { players })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload()?)
    }
}

/// Keep Alive: sent by the server (play id 0x2D) and echoed by the client
/// (play id 0x1C); a client that does not answer within 15 seconds is
/// considered gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepAlive {
    /// Arbitrary id, echoed back by the client.
    pub id: i64,
}

impl KeepAlive {
    /// Packet id of Keep Alive (play state, clientbound).
    pub const ID: u32 = 0x2D;
    /// Packet id of Keep Alive (play state, serverbound).
    pub const SERVERBOUND_ID: u32 = 0x1C;

    /// Key that resolves the clientbound Keep Alive id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:keep_alive", Self::ID);

    /// Key that resolves the serverbound Keep Alive id at runtime.
    pub const SERVERBOUND_KEY: PacketKey = PacketKey::serverbound(
        ProtocolState::Play,
        "minecraft:keep_alive",
        Self::SERVERBOUND_ID,
    );

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_i64(self.id);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            id: reader.read_i64()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Serverbound Confirm Teleportation: acknowledges Synchronize Player
/// Position; in 26.3 it also echoes the resulting position and rotation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConfirmTeleportation {
    /// Teleport id from Synchronize Player Position.
    pub teleport_id: i32,
    /// Resulting X position.
    pub x: f64,
    /// Resulting Y position.
    pub y: f64,
    /// Resulting Z position.
    pub z: f64,
    /// Resulting yaw.
    pub yaw: f32,
    /// Resulting pitch.
    pub pitch: f32,
}

impl ConfirmTeleportation {
    /// Packet id of Confirm Teleportation (play state, serverbound).
    pub const ID: u32 = 0x00;

    /// Key that resolves the Confirm Teleportation id at runtime.
    pub const KEY: PacketKey = PacketKey::serverbound(
        ProtocolState::Play,
        "minecraft:accept_teleportation",
        Self::ID,
    );

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.teleport_id);
        writer.write_f64(self.x);
        writer.write_f64(self.y);
        writer.write_f64(self.z);
        writer.write_f32(self.yaw);
        writer.write_f32(self.pitch);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            teleport_id: reader.read_varint()?,
            x: reader.read_f64()?,
            y: reader.read_f64()?,
            z: reader.read_f64()?,
            yaw: reader.read_f32()?,
            pitch: reader.read_f32()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Serverbound Player Input: the movement keys of one client tick.
///
/// The live trace (`target/parity/live-session.json`) carries this packet as
/// exactly one byte (158 packets, 20 distinct values). The bit names below
/// follow the public protocol documentation for the direction bits; sneak
/// (`0x20`) and sprint (`0x40`) are the bits the step-8 report correlates with
/// the client's crouch and run speed in that trace. `packets.json` carries no
/// field names, so the bit meanings are not established by data.
// SOURCE: live trace (payload length and byte values) + wiki (bit names);
// unverified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerInput {
    /// The input flags byte, kept verbatim.
    pub flags: u8,
}

impl PlayerInput {
    /// Packet id of Player Input (play state, serverbound).
    pub const ID: u32 = 0x2B;

    /// Key that resolves the Player Input id at runtime.
    pub const KEY: PacketKey =
        PacketKey::serverbound(ProtocolState::Play, "minecraft:player_input", Self::ID);

    /// Forward key.
    pub const FORWARD: u8 = 0x01;
    /// Backward key.
    pub const BACKWARD: u8 = 0x02;
    /// Left key.
    pub const LEFT: u8 = 0x04;
    /// Right key.
    pub const RIGHT: u8 = 0x08;
    /// Jump key.
    pub const JUMP: u8 = 0x10;
    /// Sneak (Shift) key: the sneak bit of the step-8 report.
    pub const SNEAK: u8 = 0x20;
    /// Sprint key: the sprint bit of the step-8 report.
    pub const SPRINT: u8 = 0x40;

    /// Whether the sneak bit is set: the source of truth for sneaking.
    pub fn sneak(&self) -> bool {
        self.flags & Self::SNEAK != 0
    }

    /// Whether the sprint bit is set: the source of truth for sprinting.
    pub fn sprint(&self) -> bool {
        self.flags & Self::SPRINT != 0
    }

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_u8(self.flags);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            flags: reader.read_u8()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Serverbound Player Command: a secondary signal for sneak/sprint.
///
/// The live trace carries 36 packets, all `01 01 00` or `01 02 00` — three
/// single-byte VarInts. The first is the entity id; the meaning of the second
/// and third is **not established** (`packets.json` has no field names). The
/// observed values correlate with the sprint key of [`PlayerInput`] (second
/// VarInt `1` 0–36 ms after sprint-on, `2` 0–50 ms after sprint-off, 18
/// pairs), and the Shift presses were **not** accompanied by this packet, so
/// the step-8 plan treats [`PlayerInput`] as the source of truth and this
/// packet as a secondary signal only. For the observed range `0..=2` a VarInt
/// and a single byte are indistinguishable on the wire; the trace does not
/// separate them.
// SOURCE: live trace (payload bytes `010100`/`010200`, sprint correlation) +
// wiki (field names); unverified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerCommand {
    /// Entity id this command is about (the sender's own id in the trace).
    pub entity_id: i32,
    /// Second VarInt; semantics not established (1 on sprint-on, 2 on
    /// sprint-off in the live trace).
    pub action: i32,
    /// Third VarInt; always 0 in the live trace, semantics not established.
    pub data: i32,
}

impl PlayerCommand {
    /// Packet id of Player Command (play state, serverbound).
    pub const ID: u32 = 0x2A;

    /// Key that resolves the Player Command id at runtime.
    pub const KEY: PacketKey =
        PacketKey::serverbound(ProtocolState::Play, "minecraft:player_command", Self::ID);

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.entity_id);
        writer.write_varint(self.action);
        writer.write_varint(self.data);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            entity_id: reader.read_varint()?,
            action: reader.read_varint()?,
            data: reader.read_varint()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Serverbound Punch: the client swung its arm (or attacked).
///
/// The payload is empty in the live trace: 354 packets, zero bytes each.
/// Which of the two it is (swing vs attack) is **not established** by data;
/// the step-8 plan records it as a "swing" event until the vanilla traces
/// separate the cases.
// SOURCE: live trace (zero-length payload); semantics unverified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Punch;

impl Punch {
    /// Packet id of Punch (play state, serverbound).
    pub const ID: u32 = 0x2E;

    /// Key that resolves the Punch id at runtime.
    pub const KEY: PacketKey =
        PacketKey::serverbound(ProtocolState::Play, "minecraft:punch", Self::ID);

    /// Encodes the payload (packet id excluded): always empty.
    pub fn encode_payload(&self) -> Vec<u8> {
        Vec::new()
    }

    /// Decodes the payload; the packet id must already be consumed. Any
    /// trailing byte is an error rather than a guess.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        reader.ensure_empty()?;
        Ok(Self)
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Clientbound Synchronize Player Position: teleports the client, which
/// answers with Confirm Teleportation carrying the same teleport id.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SynchronizePlayerPosition {
    /// Teleport id the client must confirm.
    pub teleport_id: i32,
    /// Target X position.
    pub x: f64,
    /// Target Y position.
    pub y: f64,
    /// Target Z position.
    pub z: f64,
    /// Velocity along X.
    pub velocity_x: f64,
    /// Velocity along Y.
    pub velocity_y: f64,
    /// Velocity along Z.
    pub velocity_z: f64,
    /// Target yaw in degrees.
    pub yaw: f32,
    /// Target pitch in degrees.
    pub pitch: f32,
    /// Teleport flags: which axes are relative (0 = fully absolute).
    pub flags: i32,
}

impl SynchronizePlayerPosition {
    /// Packet id of Synchronize Player Position (play state, clientbound).
    pub const ID: u32 = 0x49;

    /// Key that resolves the Synchronize Player Position id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:player_position", Self::ID);

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.teleport_id);
        writer.write_f64(self.x);
        writer.write_f64(self.y);
        writer.write_f64(self.z);
        writer.write_f64(self.velocity_x);
        writer.write_f64(self.velocity_y);
        writer.write_f64(self.velocity_z);
        writer.write_f32(self.yaw);
        writer.write_f32(self.pitch);
        writer.write_i32(self.flags);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            teleport_id: reader.read_varint()?,
            x: reader.read_f64()?,
            y: reader.read_f64()?,
            z: reader.read_f64()?,
            velocity_x: reader.read_f64()?,
            velocity_y: reader.read_f64()?,
            velocity_z: reader.read_f64()?,
            yaw: reader.read_f32()?,
            pitch: reader.read_f32()?,
            flags: reader.read_i32()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Clientbound Block Update: the state of one block (play id 0x08).
///
/// SOURCE: our vanilla S3 trace (`docs/BLOCK-BREAKING.md` §1.2/§1.3): after a
/// break finishes, the digger and every observer receive the removed block as
/// `packed pos (8 B) + state varint 0` (air). The S14 trace (§1.12) shows the
/// same packet carrying a non-zero state: a waterlogged slab, a seagrass, a
/// tall seagrass or a kelp is replaced by the water source (`state 89`,
/// `reports/blocks.json`), so the state is not always air. `packets.json`
/// carries only the id and direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockUpdate {
    /// Block the update is about.
    pub position: Position,
    /// New block state id (`0` = air).
    pub state: i32,
}

impl BlockUpdate {
    /// Packet id of Block Update (play state, clientbound).
    pub const ID: u32 = 0x08;

    /// Key that resolves the Block Update id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:block_update", Self::ID);

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_position(self.position);
        writer.write_varint(self.state);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            position: reader.read_position()?,
            state: reader.read_varint()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Clientbound Block Destruction: the crack stage of a block being dug (play
/// id 0x05).
///
/// SOURCE: five vanilla traces of 2026-11-14 (`docs/BLOCK-BREAKING.md` §1.3).
/// The payload is `entityId varint + packed pos (8 B) + stage u8`. The stage
/// is `floor(progress·10)` **without clamping** — the traces carry 10, 11 and
/// 13 for leaves — and [`BlockDestruction::RESET`] (`0xFF`) clears the cracks
/// when the dig stops, on completion and on an abort. The packet reaches the
/// observers of the dig, never the digger (who draws its own prediction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockDestruction {
    /// Entity id of the digging player.
    pub entity_id: i32,
    /// Block being dug.
    pub position: Position,
    /// Crack stage byte, or [`BlockDestruction::RESET`].
    pub stage: u8,
}

impl BlockDestruction {
    /// Packet id of Block Destruction (play state, clientbound).
    pub const ID: u32 = 0x05;

    /// Key that resolves the Block Destruction id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:block_destruction", Self::ID);

    /// Stage byte that removes the cracks (traced on completion and abort).
    pub const RESET: u8 = 0xFF;

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.entity_id);
        writer.write_position(self.position);
        writer.write_u8(self.stage);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            entity_id: reader.read_varint()?,
            position: reader.read_position()?,
            stage: reader.read_u8()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Clientbound Block Changed Ack: echoes the sequence of a `player_action`
/// (play id 0x04).
///
/// SOURCE: our vanilla S3 trace (`docs/BLOCK-BREAKING.md` §1.2): the digger
/// receives `01` after the start action and `02` after the finish — the
/// client's own sequence counter, carried back unchanged. `packets.json`
/// carries only the id and direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockChangedAck {
    /// Sequence echoed back from the `player_action`.
    pub sequence: i32,
}

impl BlockChangedAck {
    /// Packet id of Block Changed Ack (play state, clientbound).
    pub const ID: u32 = 0x04;

    /// Key that resolves the Block Changed Ack id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:block_changed_ack", Self::ID);

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.sequence);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            sequence: reader.read_varint()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Clientbound Set Health: the player's health, food and saturation (play
/// id 106).
///
/// SOURCE: the S7 vanilla join trace (`docs/PLAYERDATA.md` §1.3) — the packet
/// arrives right after `player_position`: `health f32 BE + food varint +
/// saturation f32 BE`. For the saved Alice the bytes were
/// `41 7a aa ab 11 00 00 00 00` = health 15.666667, food 17, saturation 0.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetHealth {
    /// Current health, 0..20 (float on the wire).
    pub health: f32,
    /// Food level (`foodLevel`), 0..20.
    pub food: i32,
    /// Food saturation (`foodSaturationLevel`).
    pub saturation: f32,
}

impl SetHealth {
    /// Packet id of Set Health (play state, clientbound).
    pub const ID: u32 = 106;

    /// Key that resolves the Set Health id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:set_health", Self::ID);

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_f32(self.health);
        writer.write_varint(self.food);
        writer.write_f32(self.saturation);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            health: reader.read_f32()?,
            food: reader.read_varint()?,
            saturation: reader.read_f32()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Clientbound Level Event: a world-wide effect tied to a position (play id
/// 0x2F).
///
/// SOURCE: our vanilla traces. The block-break event (`2001`) of the **aimed**
/// block reaches the observers of a broken block — never the digger — as
/// `event i32 BE + packed pos (8 B) + data i32 BE + one trailing byte 0x00`,
/// where `data` is the block state id the block had before it was removed
/// (S3: `data 9` = `grass_block`; S9: `data 450` = `dark_oak_leaves`; S14:
/// `data 2363` = `short_grass`, `2370` = `seagrass`, `2371`/`2372` = the two
/// halves of `tall_seagrass`, `18421`/`18424` = kelp, `15179` = the
/// waterlogged slab, `5533` = the waterlogged stairs, all from
/// `reports/blocks.json`). A **cascade** part of the break (Шаг 18, §1.12)
/// reaches the digger as well. The neighbouring break effects `2019`/`2020`
/// are **not** digger-only: in the 1644 trace both the digger and the
/// observer received 12x`2019` and 4x`2020` (data 1) for the dug position
/// (BLOCK-BREAKING.md §1.9). We do not send them (16a.5).
///
/// The trailing byte is **not established**: the traces always carry `0x00`,
/// so we reproduce it as a constant [hypothesis: a global-event flag].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelEvent {
    /// Effect code (`2001` = block break with particles).
    pub event: i32,
    /// Position the effect is tied to.
    pub position: Position,
    /// Event data — for a break, the removed block's state id.
    pub data: i32,
    /// Traced constant `0x00`; its meaning is not established (see above).
    pub trailing: u8,
}

impl LevelEvent {
    /// Packet id of Level Event (play state, clientbound).
    pub const ID: u32 = 0x2F;

    /// Key that resolves the Level Event id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:level_event", Self::ID);

    /// Event code of the block-break effect, from the S3/S9 traces.
    pub const EVENT_BLOCK_BREAK: i32 = 2001;

    /// The trailing byte the traces always carry (not established, see the
    /// struct comment).
    pub const TRACED_TRAILING: u8 = 0x00;

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_i32(self.event);
        writer.write_position(self.position);
        writer.write_i32(self.data);
        writer.write_u8(self.trailing);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            event: reader.read_i32()?,
            position: reader.read_position()?,
            data: reader.read_i32()?,
            trailing: reader.read_u8()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// Serverbound Player Action: the client's block-breaking signal (play id
/// 0x29).
///
/// SOURCE: our vanilla S3 trace (`docs/BLOCK-BREAKING.md` §1.2): a broken
/// block arrives as two packets with the same packed position and face — a
/// start (`00`) and a finish (`03`) — and the finish is followed by a
/// `block_update` to the removed state and a `block_changed_ack` echoing the
/// sequence. The abort value (`02`) is **behaviourally inferred** from a
/// trace where a short tap produced an ack and left the block in place; the
/// S3 trace does not carry that byte itself. `face` is a byte (`01`) in every
/// observation; other values are not established. `packets.json` carries only
/// the id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerAction {
    /// Action varint: see [`PlayerAction::ACTION_START`],
    /// [`PlayerAction::ACTION_ABORT`] and [`PlayerAction::ACTION_FINISH`].
    pub action: i32,
    /// Target block.
    pub position: Position,
    /// Face byte of the clicked block face; `01` in every trace packet.
    pub face: u8,
    /// The client's own sequence counter, echoed by `block_changed_ack`.
    pub sequence: i32,
}

impl PlayerAction {
    /// Packet id of Player Action (play state, serverbound).
    pub const ID: u32 = 0x29;

    /// Key that resolves the Player Action id at runtime.
    pub const KEY: PacketKey =
        PacketKey::serverbound(ProtocolState::Play, "minecraft:player_action", Self::ID);

    /// Action value the S3 trace records when breaking starts.
    pub const ACTION_START: i32 = 0x00;
    /// Action value inferred for an aborted break (not byte-confirmed by the
    /// S3 trace — see the struct docs).
    pub const ACTION_ABORT: i32 = 0x02;
    /// Action value the S3 trace records when breaking finishes.
    pub const ACTION_FINISH: i32 = 0x03;

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.action);
        writer.write_position(self.position);
        writer.write_u8(self.face);
        writer.write_varint(self.sequence);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            action: reader.read_varint()?,
            position: reader.read_position()?,
            face: reader.read_u8()?,
            sequence: reader.read_varint()?,
        })
    }

    /// Encodes the packet as a complete frame.
    pub fn to_frame(&self) -> Result<Vec<u8>, CodecError> {
        encode_frame(Self::ID, &self.encode_payload())
    }
}

/// One item stack of the 1.21.5+ wire format (Шаг 24a).
///
/// SOURCE: our vanilla S24a trace (`docs/BLOCK-PLACING.md` §2/§5): a stack is
/// `count VarInt` (`0` = empty, and then nothing follows), and for a non-empty
/// stack `item id VarInt` + `added components VarInt` + the components + `removed
/// components VarInt`. Every stack this client sent or received in the traces
/// carried `0` added and `0` removed components (stone in slot 36 was
/// `40 01 00 00`), so the component payloads are kept **verbatim**: the codec
/// does not know the component schemas and must not re-encode bytes the sender
/// chose. For the same reason a stack is only decoded to the end of the payload
/// (the `set_creative_mode_slot` field it follows is the packet's last field).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemStack {
    /// Stack size; `0` is an empty stack.
    pub count: i32,
    /// Item protocol id (`minecraft:item`); `0` when the stack is empty.
    pub item: i32,
    /// Raw bytes after the item id: the added-component list and the removed
    /// component list, as the sender wrote them (`00 00` for no components).
    pub components: Vec<u8>,
}

impl ItemStack {
    /// The empty stack.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Whether the stack is empty.
    pub fn is_empty(&self) -> bool {
        self.count <= 0
    }

    /// A stack of `count` of `item` with no components.
    pub fn new(item: i32, count: i32) -> Self {
        Self {
            count,
            item,
            components: vec![0, 0],
        }
    }

    /// Appends the stack to `writer`.
    pub fn encode(&self, writer: &mut PacketWriter) {
        if self.is_empty() {
            writer.write_varint(0);
            return;
        }
        writer.write_varint(self.count);
        writer.write_varint(self.item);
        // An empty tail means "no components": the two zero counts the wire
        // needs are written here rather than guessed by every caller.
        if self.components.is_empty() {
            writer.write_varint(0);
            writer.write_varint(0);
        } else {
            writer.write_bytes(&self.components);
        }
    }

    /// Decodes a stack from `reader`, taking every remaining byte as the
    /// component tail (see the struct docs for why).
    pub fn decode_tail(reader: &mut PacketReader<'_>, limit: usize) -> Result<Self, CodecError> {
        let count = reader.read_varint()?;
        if count <= 0 {
            return Ok(Self::empty());
        }
        let item = reader.read_varint()?;
        let remaining = reader.remaining();
        if remaining > limit {
            return Err(CodecError::Unsupported {
                context: "item stack components larger than the sanity cap",
            });
        }
        let components = reader.read_bytes(remaining)?.to_vec();
        Ok(Self {
            count,
            item,
            components,
        })
    }
}

/// Maximum component tail of one stack: the `set_creative_mode_slot` payload is
/// small in every trace (6..7 bytes), so a sanity cap keeps a hostile client
/// from parking arbitrary bytes in the inventory.
const MAX_ITEM_COMPONENTS: usize = 1024;

/// Serverbound Set Creative Mode Slot (play id 57): a creative-mode client
/// sets one player-inventory slot.
///
/// SOURCE: the S24a vanilla trace (`docs/BLOCK-PLACING.md` §2): `slot i16`
/// big-endian + one [`ItemStack`]; there is no container id on the wire, so the
/// slot is a player-inventory menu slot (36..44 hotbar). The trace carries no
/// clientbound answer to this packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetCreativeModeSlot {
    /// Player-inventory menu slot (`0..45`).
    pub slot: i16,
    /// Item written into the slot (possibly empty).
    pub item: ItemStack,
}

impl SetCreativeModeSlot {
    /// Packet id of Set Creative Mode Slot (play state, serverbound).
    pub const ID: u32 = 57;

    /// Key that resolves the Set Creative Mode Slot id at runtime.
    pub const KEY: PacketKey = PacketKey::serverbound(
        ProtocolState::Play,
        "minecraft:set_creative_mode_slot",
        Self::ID,
    );

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_u16(self.slot as u16);
        self.item.encode(&mut writer);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        let slot = reader.read_u16()? as i16;
        let item = ItemStack::decode_tail(reader, MAX_ITEM_COMPONENTS)?;
        Ok(Self { slot, item })
    }
}

/// Serverbound Set Carried Item (play id 54): the selected hotbar slot.
///
/// SOURCE: the S24a vanilla trace (§2/§4): `slot i16` big-endian, and no
/// clientbound answer — the wheel and the keys 1..9 both send only this packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetCarriedItem {
    /// Selected hotbar slot, `0..8`.
    pub slot: i16,
}

impl SetCarriedItem {
    /// Packet id of Set Carried Item (play state, serverbound).
    pub const ID: u32 = 54;

    /// Key that resolves the Set Carried Item id at runtime.
    pub const KEY: PacketKey =
        PacketKey::serverbound(ProtocolState::Play, "minecraft:set_carried_item", Self::ID);

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_u16(self.slot as u16);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            slot: reader.read_u16()? as i16,
        })
    }
}

/// Serverbound Pick Item From Block (play id 36): creative "pick block".
///
/// SOURCE: the S17 vanilla trace (`docs/BLOCK-PLACING.md` §2): `position` (8 B
/// packed) + `include_data` bool (`00` in every observation). The server answers
/// with `set_held_slot` and, when the item was not already in the hotbar,
/// `container_set_slot` (§5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PickItemFromBlock {
    /// Clicked block.
    pub position: Position,
    /// `true` when the client asks for the block's data (state-dependent pick).
    pub include_data: bool,
}

impl PickItemFromBlock {
    /// Packet id of Pick Item From Block (play state, serverbound).
    pub const ID: u32 = 36;

    /// Key that resolves the Pick Item From Block id at runtime.
    pub const KEY: PacketKey = PacketKey::serverbound(
        ProtocolState::Play,
        "minecraft:pick_item_from_block",
        Self::ID,
    );

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_position(self.position);
        writer.write_bool(self.include_data);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            position: reader.read_position()?,
            include_data: reader.read_bool()?,
        })
    }
}

/// Serverbound Use Item On (play id 66): the "right click a block" packet that
/// places a block (Шаг 24b).
///
/// SOURCE: the S17 and S24b vanilla traces (`docs/BLOCK-PLACING.md` §2): `hand
/// VarInt`, `position` (8 B packed), `face VarInt`, cursor `f32 ×3`, `inside`
/// bool, `world_border` bool, `sequence VarInt`. The face values observed:
/// `1` = up, `2` = north, `5` = east (the S17 log axis table fills the rest:
/// `0` = down, `3` = south, `4` = west).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UseItemOn {
    /// Hand: `0` main, `1` off hand (only `0` observed).
    pub hand: i32,
    /// Clicked block.
    pub position: Position,
    /// Clicked face (`0..5`, see the struct docs).
    pub face: i32,
    /// Hit point inside the block, `0.0..1.0` per axis.
    pub cursor_x: f32,
    /// Hit point inside the block, `0.0..1.0` per axis.
    pub cursor_y: f32,
    /// Hit point inside the block, `0.0..1.0` per axis.
    pub cursor_z: f32,
    /// Whether the player's own bounding box intersects the clicked block.
    pub inside_block: bool,
    /// Whether the interaction crosses the world border.
    pub world_border: bool,
    /// The client's own counter, echoed back in `block_changed_ack`.
    pub sequence: i32,
}

impl UseItemOn {
    /// Packet id of Use Item On (play state, serverbound).
    pub const ID: u32 = 66;

    /// Key that resolves the Use Item On id at runtime.
    pub const KEY: PacketKey =
        PacketKey::serverbound(ProtocolState::Play, "minecraft:use_item_on", Self::ID);

    /// Face values of [`UseItemOn::face`].
    pub const FACE_DOWN: i32 = 0;
    /// See [`UseItemOn::FACE_DOWN`].
    pub const FACE_UP: i32 = 1;
    /// See [`UseItemOn::FACE_DOWN`].
    pub const FACE_NORTH: i32 = 2;
    /// See [`UseItemOn::FACE_DOWN`].
    pub const FACE_SOUTH: i32 = 3;
    /// See [`UseItemOn::FACE_DOWN`].
    pub const FACE_WEST: i32 = 4;
    /// See [`UseItemOn::FACE_DOWN`].
    pub const FACE_EAST: i32 = 5;

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.hand);
        writer.write_position(self.position);
        writer.write_varint(self.face);
        writer.write_f32(self.cursor_x);
        writer.write_f32(self.cursor_y);
        writer.write_f32(self.cursor_z);
        writer.write_bool(self.inside_block);
        writer.write_bool(self.world_border);
        writer.write_varint(self.sequence);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            hand: reader.read_varint()?,
            position: reader.read_position()?,
            face: reader.read_varint()?,
            cursor_x: reader.read_f32()?,
            cursor_y: reader.read_f32()?,
            cursor_z: reader.read_f32()?,
            inside_block: reader.read_bool()?,
            world_border: reader.read_bool()?,
            sequence: reader.read_varint()?,
        })
    }
}

/// Clientbound Container Set Content (play id 18): the whole content of a
/// window, sent for the player inventory at join (Шаг 24a).
///
/// SOURCE: the S24a vanilla trace (§5): `window id VarInt 0`, `state id VarInt
/// 1`, `count VarInt 46`, 46 item stacks, the carried stack — 50 bytes for an
/// empty inventory (`00 01 2e` + 46 × `00` + `00`). The window id is zero in
/// every trace, so a VarInt and a byte encode identically; the public protocol
/// document lists it as a VarInt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSetContent {
    /// Window id (`0` = the player inventory).
    pub window: i32,
    /// Window state id the client must echo in clicks.
    pub state_id: i32,
    /// Every slot of the window, in menu order.
    pub items: Vec<ItemStack>,
    /// The stack held on the cursor.
    pub carried: ItemStack,
}

impl ContainerSetContent {
    /// Packet id of Container Set Content (play state, clientbound).
    pub const ID: u32 = 18;

    /// Key that resolves the Container Set Content id at runtime.
    pub const KEY: PacketKey = PacketKey::clientbound(
        ProtocolState::Play,
        "minecraft:container_set_content",
        Self::ID,
    );

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.window);
        writer.write_varint(self.state_id);
        writer.write_varint(self.items.len() as i32);
        for item in &self.items {
            item.encode(&mut writer);
        }
        self.carried.encode(&mut writer);
        writer.into_bytes()
    }
}

/// Clientbound Container Set Slot (play id 20): one window slot changed
/// (Шаг 24a — the answer to `pick_item_from_block`).
///
/// SOURCE: the S17 vanilla trace (§5): `window byte 0`, `state id VarInt`,
/// `slot i16` big-endian, item stack. Stone into hotbar slot 36 was
/// `00 02 0024 01 01 00 00`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSetSlot {
    /// Window id (`0` = the player inventory).
    pub window: u8,
    /// Window state id after the change.
    pub state_id: i32,
    /// Player-inventory menu slot (`36..44` = hotbar).
    pub slot: i16,
    /// New stack of the slot.
    pub item: ItemStack,
}

impl ContainerSetSlot {
    /// Packet id of Container Set Slot (play state, clientbound).
    pub const ID: u32 = 20;

    /// Key that resolves the Container Set Slot id at runtime.
    pub const KEY: PacketKey = PacketKey::clientbound(
        ProtocolState::Play,
        "minecraft:container_set_slot",
        Self::ID,
    );

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_u8(self.window);
        writer.write_varint(self.state_id);
        writer.write_u16(self.slot as u16);
        self.item.encode(&mut writer);
        writer.into_bytes()
    }
}

/// Clientbound Set Held Slot (play id 107): the selected hotbar slot.
///
/// SOURCE: the S24a/S17 vanilla traces (§4/§5): a single VarInt (`00` .. `08`).
/// Vanilla sends it at join and after every creative pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetHeldSlot {
    /// Selected hotbar slot, `0..8`.
    pub slot: i32,
}

impl SetHeldSlot {
    /// Packet id of Set Held Slot (play state, clientbound).
    pub const ID: u32 = 107;

    /// Key that resolves the Set Held Slot id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:set_held_slot", Self::ID);

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.slot);
        writer.into_bytes()
    }
}

/// Maximum length of a chat message or a command, in characters.
///
/// SOURCE: the vanilla 26.3 client refuses longer input; every S19/S21 trace
/// message is far below it.
const MAX_CHAT_CHARS: usize = 256;

/// Key of the clientbound command tree in the play state (Step 25b).
pub const COMMANDS_TREE: PacketKey =
    PacketKey::clientbound(ProtocolState::Play, "minecraft:commands", 0x10);

/// Serverbound Chat (play id 9): one player chat message (Step 25a).
///
/// Only the text is decoded. The S19 trace shows `timestamp i64`, `salt i64`,
/// an absent signature option, a message count and an acknowledged bitset
/// after the text; in offline mode the server ignores all of them, and their
/// exact split is not needed to deliver the line.
// SOURCE: 2026-11-18_s19s20-vanilla-step25-r1, conn-1 seq 62417.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// The typed text.
    pub message: String,
}

impl ChatMessage {
    /// Packet id of Chat (play state, serverbound).
    pub const ID: u32 = 0x09;

    /// Key that resolves the Chat id at runtime.
    pub const KEY: PacketKey =
        PacketKey::serverbound(ProtocolState::Play, "minecraft:chat", Self::ID);

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            message: reader.read_string(MAX_CHAT_CHARS)?,
        })
    }

    /// Encodes the payload (packet id excluded), for tests.
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_string(&self.message, MAX_CHAT_CHARS)?;
        Ok(writer.into_bytes())
    }
}

/// Serverbound Chat Command (play id 7): one command typed by the player
/// (Step 25a/25b), without the leading slash.
///
/// SOURCE: 2026-11-18_s21-l2-vanilla-step25-r2, conn-1 seq 78557: a single
/// String (`11 "gamemode creative"`), no trailing bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatCommand {
    /// Command text, without the leading `/`.
    pub command: String,
}

impl ChatCommand {
    /// Packet id of Chat Command (play state, serverbound).
    pub const ID: u32 = 0x07;

    /// Key that resolves the Chat Command id at runtime.
    pub const KEY: PacketKey =
        PacketKey::serverbound(ProtocolState::Play, "minecraft:chat_command", Self::ID);

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            command: reader.read_string(MAX_CHAT_CHARS)?,
        })
    }
}

/// Serverbound Chat Command Signed (play id 8): the signed variant of
/// [`ChatCommand`]. Offline clients use id 7; the S19 trace carries none.
///
/// The text is the first field in both variants, so the same decoder serves
/// both and the signature tail is ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatCommandSigned {
    /// Command text, without the leading `/`.
    pub command: String,
}

impl ChatCommandSigned {
    /// Packet id of Chat Command Signed (play state, serverbound).
    pub const ID: u32 = 0x08;

    /// Key that resolves the Chat Command Signed id at runtime.
    pub const KEY: PacketKey = PacketKey::serverbound(
        ProtocolState::Play,
        "minecraft:chat_command_signed",
        Self::ID,
    );

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            command: reader.read_string(MAX_CHAT_CHARS)?,
        })
    }
}

/// Clientbound Player Chat (play id 66): one chat line, sent to every client
/// including the sender (Step 25a).
///
/// The packet is reproduced field for field from the S19 trace except the
/// sender-name component, whose entries our NBT map stores sorted:
///
/// - `global_index` VarInt, `sender` UUID, `index` VarInt (`0`), an absent
///   signature option (`00`), the text, `timestamp` i64 and `salt` i64;
/// - four constant bytes `00 00 00 01`: the trace does not establish their
///   field split (unsigned content absent, filter type, chat type). They are
///   reproduced verbatim and documented as unverified;
/// - the sender-name component (text/insertion/click/hover with the sender's
///   UUID), then an absent target-name option.
///
/// SOURCE: 2026-11-18_s19s20-vanilla-step25-r1, conn-1 seq 62427 (258 bytes
/// for a 16-character message from `ChatA`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerChat {
    /// Server-wide message counter.
    pub global_index: i32,
    /// Offline UUID of the sender.
    pub sender: Uuid,
    /// The message text.
    pub content: String,
    /// Milliseconds since the Unix epoch.
    pub timestamp: i64,
    /// The rendered sender name (see [`PlayerChat::name_component`]).
    pub sender_name: NbtTag,
}

impl PlayerChat {
    /// Packet id of Player Chat (play state, clientbound).
    pub const ID: u32 = 0x42;

    /// Key that resolves the Player Chat id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:player_chat", Self::ID);

    /// The four bytes between the salt and the sender-name component.
    ///
    /// SOURCE: the S19 trace (identical in both messages); semantics not
    /// established.
    const TRACED_TAIL: [u8; 4] = [0x00, 0x00, 0x00, 0x01];

    /// The sender-name component the vanilla trace carries: the name as text
    /// and insertion, `/tell <name> ` on click and the player entity on hover.
    ///
    /// SOURCE: the S19 trace NBT bytes, decoded field by field.
    pub fn name_component(name: &str, uuid: Uuid) -> NbtTag {
        let chunks = (0..4)
            .map(|i| {
                i32::from_be_bytes([
                    uuid[i * 4],
                    uuid[i * 4 + 1],
                    uuid[i * 4 + 2],
                    uuid[i * 4 + 3],
                ])
            })
            .collect();
        let mut click = BTreeMap::new();
        click.insert(
            "action".to_owned(),
            NbtTag::String("suggest_command".to_owned()),
        );
        click.insert(
            "command".to_owned(),
            NbtTag::String(format!("/tell {name} ")),
        );
        let mut hover = BTreeMap::new();
        hover.insert(
            "action".to_owned(),
            NbtTag::String("show_entity".to_owned()),
        );
        hover.insert(
            "id".to_owned(),
            NbtTag::String("minecraft:player".to_owned()),
        );
        hover.insert("name".to_owned(), NbtTag::String(name.to_owned()));
        hover.insert("uuid".to_owned(), NbtTag::IntArray(chunks));
        let mut root = BTreeMap::new();
        root.insert("click_event".to_owned(), NbtTag::Compound(click));
        root.insert("hover_event".to_owned(), NbtTag::Compound(hover));
        root.insert("insertion".to_owned(), NbtTag::String(name.to_owned()));
        root.insert("text".to_owned(), NbtTag::String(name.to_owned()));
        NbtTag::Compound(root)
    }

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_varint(self.global_index);
        writer.write_uuid(self.sender);
        writer.write_varint(0); // message index inside the sender's chain
        writer.write_bool(false); // no signature (offline mode)
        writer.write_string(&self.content, MAX_CHAT_CHARS)?;
        writer.write_i64(self.timestamp);
        writer.write_i64(0); // salt
        writer.write_bytes(&Self::TRACED_TAIL);
        writer.write_nbt(&self.sender_name)?;
        writer.write_bool(false); // no target name
        Ok(writer.into_bytes())
    }
}

/// Clientbound System Chat (play id 124): a component plus the overlay flag
/// (Step 25a). Join/leave lines, command replies and errors use it.
///
/// SOURCE: 2026-11-18_s20-vanilla-step25-r4 and S21 replies: a nameless NBT
/// component followed by one `00` overlay byte.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemChat {
    /// The message component.
    pub component: NbtTag,
    /// Whether the line is drawn in the action bar instead of the chat.
    pub overlay: bool,
}

impl SystemChat {
    /// Packet id of System Chat (play state, clientbound).
    pub const ID: u32 = 0x7C;

    /// Key that resolves the System Chat id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:system_chat", Self::ID);

    /// The line vanilla sends when a player joins or leaves (Шаг 25a): the
    /// `multiplayer.player.joined`/`multiplayer.player.left` translation in
    /// yellow with the player's name component as its only argument.
    ///
    /// SOURCE: 2026-11-18_s20-vanilla-step25-r4, conn-1 seq 49174 (joined)
    /// and 57493 (left); the compound key order is our sorted map's, NBT does
    /// not define one.
    pub fn join_leave(key: &str, name: &str, uuid: Uuid) -> Self {
        let mut root = BTreeMap::new();
        root.insert("color".to_owned(), NbtTag::String("yellow".to_owned()));
        root.insert("translate".to_owned(), NbtTag::String(key.to_owned()));
        root.insert(
            "with".to_owned(),
            NbtTag::List(vec![PlayerChat::name_component(name, uuid)]),
        );
        Self {
            component: NbtTag::Compound(root),
            overlay: false,
        }
    }

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Result<Vec<u8>, CodecError> {
        let mut writer = PacketWriter::new();
        writer.write_nbt(&self.component)?;
        writer.write_bool(self.overlay);
        Ok(writer.into_bytes())
    }
}

/// Clientbound Player Abilities (play id 65): the actor's ability flags and
/// speeds, sent twice around a game-mode change (Step 25c).
///
/// SOURCE: 2026-11-18_s21-l2-vanilla-step25-r2: `0d 3d4ccccd 3dcccccd`
/// (creative), `00 3d4ccccd 3dcccccd` (survival/adventure), `07 ...`
/// (spectator).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerAbilities {
    /// Ability flags: `0x01` invulnerable, `0x02` flying, `0x04` may fly,
    /// `0x08` creative.
    pub flags: u8,
    /// Flying speed (`0.05` in every trace).
    pub flying_speed: f32,
    /// Walking speed (`0.1` in every trace).
    pub walking_speed: f32,
}

impl PlayerAbilities {
    /// Packet id of Player Abilities (play state, clientbound).
    pub const ID: u32 = 0x41;

    /// Key that resolves the Player Abilities id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:player_abilities", Self::ID);

    /// The traced flags and speeds for a game mode.
    ///
    /// SOURCE: S21 l2: creative `0x0d`, survival/adventure `0x00`, spectator
    /// `0x07`; speeds `0.05` and `0.1` in all four.
    pub fn for_mode(game_mode: i32) -> Self {
        let flags = match game_mode {
            1 => 0x0D,
            3 => 0x07,
            _ => 0x00,
        };
        Self {
            flags,
            flying_speed: 0.05,
            walking_speed: 0.1,
        }
    }

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_u8(self.flags);
        writer.write_f32(self.flying_speed);
        writer.write_f32(self.walking_speed);
        writer.into_bytes()
    }
}

/// Clientbound Player Info Update carrying only the game mode action mask
/// (`0x04`), as vanilla sends on a game-mode change (Step 25c).
///
/// SOURCE: 2026-11-18_s21-l2-vanilla-step25-r2, conn-1 seq 78584: `04 01` +
/// UUID + mode VarInt = 19 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerInfoGameMode {
    /// Player whose mode changed.
    pub uuid: Uuid,
    /// New game mode (`0..=3`).
    pub game_mode: i32,
}

impl PlayerInfoGameMode {
    /// Packet id of Player Info Update (play state, clientbound).
    pub const ID: u32 = 0x47;

    /// Key that resolves the Player Info Update id at runtime.
    pub const KEY: PacketKey = PacketKey::clientbound(
        ProtocolState::Play,
        "minecraft:player_info_update",
        Self::ID,
    );

    /// Action mask of the mode-only update.
    const ACTIONS: u8 = 0x04;

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_u8(Self::ACTIONS);
        writer.write_varint(1);
        writer.write_uuid(self.uuid);
        writer.write_varint(self.game_mode);
        writer.into_bytes()
    }
}

/// One clock update of [`SetTime`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockUpdate {
    /// Clock registry id (`0` = `minecraft:overworld` in the trace).
    pub id: i32,
    /// Total ticks of the clock (day count × 24000 + time of day).
    pub total_ticks: i32,
    /// Partial tick (`0.0` in every trace).
    pub partial_tick: f32,
    /// Clock rate (`1.0` in every trace).
    pub rate: f32,
}

/// Clientbound Set Time (play id 115): the world game time plus per-clock
/// updates (Step 25b `/time set day`).
///
/// The layout was derived from the traces: `game_time i64`, a VarInt count,
/// then per clock a VarInt id, a VarInt total-ticks value, an f32 partial
/// tick and an f32 rate. The 31-byte join packet (two clocks) and the 21-byte
/// `/time set day` packet (one clock, 25000 ticks) both decode exactly under
/// it.
// SOURCE: 2026-11-18_s21-l2-vanilla-step25-r2, conn-1 seq 60 and 106749.
#[derive(Debug, Clone, PartialEq)]
pub struct SetTime {
    /// World game time in ticks.
    pub game_time: i64,
    /// Clock updates of this packet.
    pub clocks: Vec<ClockUpdate>,
}

impl SetTime {
    /// Packet id of Set Time (play state, clientbound).
    pub const ID: u32 = 115;

    /// Key that resolves the Set Time id at runtime.
    pub const KEY: PacketKey =
        PacketKey::clientbound(ProtocolState::Play, "minecraft:set_time", Self::ID);

    /// Encodes the payload (packet id excluded).
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_i64(self.game_time);
        writer.write_varint(self.clocks.len() as i32);
        for clock in &self.clocks {
            writer.write_varint(clock.id);
            writer.write_varint(clock.total_ticks);
            writer.write_f32(clock.partial_tick);
            writer.write_f32(clock.rate);
        }
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        let game_time = reader.read_i64()?;
        let count = super::read_count(reader)?;
        let mut clocks = Vec::with_capacity(count.min(8));
        for _ in 0..count {
            clocks.push(ClockUpdate {
                id: reader.read_varint()?,
                total_ticks: reader.read_varint()?,
                partial_tick: reader.read_f32()?,
                rate: reader.read_f32()?,
            });
        }
        Ok(Self { game_time, clocks })
    }
}

/// Reads a non-negative length prefix used by the array fields above.
fn read_count(reader: &mut PacketReader<'_>) -> Result<usize, CodecError> {
    let count = reader.read_varint()?;
    usize::try_from(count).map_err(|_| CodecError::NegativeLength(count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::split_frame;
    use crate::packets::login::offline_uuid;
    use crate::packets::test_support::hex;

    /// Decodes `frame`, checks the packet id and returns the reader.
    macro_rules! decoded {
        ($frame:expr, $id:expr) => {{
            let (id, reader) = split_frame(&$frame).expect("splittable");
            assert_eq!(id, $id);
            reader
        }};
    }

    #[test]
    fn login_play_roundtrips_all_fields() {
        let packet = LoginPlay {
            entity_id: 1,
            hardcore: false,
            dimension_names: vec!["minecraft:overworld".to_owned()],
            max_players: 20,
            view_distance: 10,
            simulation_distance: 10,
            reduced_debug_info: false,
            enable_respawn_screen: true,
            do_limited_crafting: false,
            dimension_type: 0,
            dimension_name: "minecraft:overworld".to_owned(),
            hashed_seed: 0,
            game_mode: 1,
            previous_game_mode: 0,
            is_debug: false,
            is_flat: false,
            death_location: None,
            portal_cooldown: 0,
            sea_level: 63,
            online_mode: false,
            enforces_secure_chat: false,
        };
        let frame = packet.to_frame().expect("encodable");
        let mut reader = decoded!(frame, LoginPlay::ID);

        let decoded = LoginPlay::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn login_play_roundtrips_a_death_location() {
        let packet = LoginPlay {
            entity_id: 7,
            hardcore: true,
            dimension_names: vec!["minecraft:overworld".to_owned()],
            max_players: 10,
            view_distance: 8,
            simulation_distance: 8,
            reduced_debug_info: true,
            enable_respawn_screen: false,
            do_limited_crafting: true,
            dimension_type: 3,
            dimension_name: "minecraft:the_nether".to_owned(),
            hashed_seed: 0x0123_4567_89AB_CDEF,
            game_mode: 2,
            previous_game_mode: 4,
            is_debug: true,
            is_flat: true,
            death_location: Some(DeathLocation {
                dimension: "minecraft:the_nether".to_owned(),
                position: Position {
                    x: -100,
                    y: 64,
                    z: 200,
                },
            }),
            portal_cooldown: 120,
            sea_level: 63,
            online_mode: true,
            enforces_secure_chat: true,
        };
        let frame = packet.to_frame().expect("encodable");
        let mut reader = decoded!(frame, LoginPlay::ID);

        let decoded = LoginPlay::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }

    /// The hashed seed formula is read off the vanilla join recording: the
    /// parity world has `level-seed=12345`, and the vanilla `minecraft:login`
    /// payload carries exactly `hashed_seed(12345)` as its 8 seed bytes.
    #[test]
    fn hashed_seed_matches_the_vanilla_recording() {
        // Wire bytes of the recording: 04 13 91 24 51 35 54 e1.
        assert_eq!(hashed_seed(12345), 0x0413_9124_5135_54E1);
        // The hash must actually depend on the seed.
        assert_ne!(hashed_seed(12345), hashed_seed(12346));
    }

    #[test]
    fn game_event_carries_an_ubyte_and_a_float() {
        let packet = GameEvent {
            event: GameEvent::START_WAITING_FOR_CHUNKS,
            value: 0.0,
        };
        let frame = packet.to_frame().expect("encodable");
        let mut reader = decoded!(frame, GameEvent::ID);

        let decoded = GameEvent::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        assert_eq!(decoded.event, 13);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn set_center_chunk_roundtrips_chunk_coordinates() {
        let packet = SetCenterChunk {
            chunk_x: -12,
            chunk_z: 34,
        };
        let frame = packet.to_frame().expect("encodable");
        let mut reader = decoded!(frame, SetCenterChunk::ID);

        let decoded = SetCenterChunk::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }

    /// `forget_level_chunk` encodes the packed `ChunkPos`: z in the high word
    /// and x in the low word, byte for byte the payloads of the vanilla 26.3
    /// walking trace `window-p4.jsonl` (seq 16000 and 16022); negative
    /// coordinates round-trip too.
    #[test]
    fn forget_level_chunk_matches_the_traced_payloads() {
        // seq 16000: packed (x, z) = (-4, -8) → ff ff ff f8 ff ff ff fc.
        let traced_negative = [0xFF, 0xFF, 0xFF, 0xF8, 0xFF, 0xFF, 0xFF, 0xFC];
        // seq 16022: packed (x, z) = (1, 9) → 00 00 00 09 00 00 00 01.
        let traced_positive = [0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x01];
        let cases = [
            (
                ForgetLevelChunk {
                    chunk_x: -4,
                    chunk_z: -8,
                },
                traced_negative,
            ),
            (
                ForgetLevelChunk {
                    chunk_x: 1,
                    chunk_z: 9,
                },
                traced_positive,
            ),
        ];
        for (packet, bytes) in cases {
            assert_eq!(packet.encode_payload(), bytes, "encode {packet:?}");
            let mut reader = PacketReader::new(&bytes);
            let decoded = ForgetLevelChunk::decode_payload(&mut reader).expect("decodable");
            assert_eq!(decoded, packet);
            reader.ensure_empty().expect("no trailing bytes");

            // Through a frame: the id byte, then the same 8 payload bytes.
            let frame = packet.to_frame().expect("encodable");
            let mut reader = decoded!(frame, ForgetLevelChunk::ID);
            let decoded = ForgetLevelChunk::decode_payload(&mut reader).expect("decodable");
            assert_eq!(decoded, packet);
            reader.ensure_empty().expect("no trailing bytes");
        }
    }

    #[test]
    fn spawn_position_roundtrips_dimension_and_position() {
        let packet = SetDefaultSpawnPosition {
            dimension: "minecraft:overworld".to_owned(),
            position: Position { x: 8, y: 64, z: 8 },
            yaw: 0.0,
            pitch: 0.0,
        };
        let frame = packet.to_frame().expect("encodable");
        let mut reader = decoded!(frame, SetDefaultSpawnPosition::ID);

        let decoded = SetDefaultSpawnPosition::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn player_info_update_roundtrips_the_join_actions() {
        let packet = PlayerInfoUpdate {
            players: vec![PlayerInfoEntry {
                uuid: offline_uuid("Steve"),
                name: "Steve".to_owned(),
                properties: Vec::new(),
                game_mode: 1,
                listed: true,
                latency: 0,
            }],
        };
        let frame = packet.to_frame().expect("encodable");
        let mut reader = decoded!(frame, PlayerInfoUpdate::ID);

        let decoded = PlayerInfoUpdate::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn player_info_update_rejects_unsupported_actions() {
        // Only the four join actions are modelled; any other mask must not
        // be silently mis-decoded. Byte 0x02 is "Initialize Chat".
        let mut payload = vec![0x02, 0x00];
        payload.extend_from_slice(&[0; 8]);
        let mut reader = PacketReader::new(&payload);
        let err = PlayerInfoUpdate::decode_payload(&mut reader).expect_err("mask");
        assert!(matches!(err, CodecError::TrailingBytes { .. }), "{err}");
    }

    #[test]
    fn keep_alive_roundtrips_its_id() {
        let packet = KeepAlive {
            id: 0x0123_4567_89AB_CDEF,
        };
        let frame = packet.to_frame().expect("encodable");
        let mut reader = decoded!(frame, KeepAlive::ID);

        let decoded = KeepAlive::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
        assert_ne!(KeepAlive::ID, KeepAlive::SERVERBOUND_ID);
    }

    #[test]
    fn confirm_teleportation_roundtrips_the_echo() {
        let packet = ConfirmTeleportation {
            teleport_id: 1,
            x: 8.5,
            y: 64.0,
            z: 8.5,
            yaw: 0.0,
            pitch: 0.0,
        };
        let payload = packet.encode_payload();
        let mut reader = PacketReader::new(&payload);

        let decoded = ConfirmTeleportation::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }

    #[test]
    fn player_input_decodes_the_one_byte_traces() {
        // Every observed payload of the live trace: one byte, 20 values.
        for flags in [
            0x00, 0x01, 0x02, 0x04, 0x05, 0x06, 0x08, 0x09, 0x0a, 0x11, 0x20, 0x21, 0x24, 0x25,
            0x40, 0x41, 0x44, 0x45, 0x49, 0x51,
        ] {
            let packet = PlayerInput { flags };
            let payload = packet.encode_payload();
            assert_eq!(payload, vec![flags], "the trace carries exactly one byte");
            let mut reader = PacketReader::new(&payload);
            let decoded = PlayerInput::decode_payload(&mut reader).expect("decodable");
            assert_eq!(decoded, packet);
            reader.ensure_empty().expect("no trailing bytes");
        }
    }

    #[test]
    fn player_input_names_the_sneak_and_sprint_bits() {
        // The live trace shows 0x20 with a Shift press and 0x40 while the
        // client ran; the bits are named in PlayerInput and asserted here so
        // a change of the layout has to touch this test.
        let input = PlayerInput { flags: 0x20 };
        assert!(input.sneak());
        assert!(!input.sprint());
        let input = PlayerInput { flags: 0x40 };
        assert!(!input.sneak());
        assert!(input.sprint());
        let input = PlayerInput { flags: 0x60 };
        assert!(input.sneak());
        assert!(input.sprint());
    }

    #[test]
    fn player_command_roundtrips_the_two_observed_payloads() {
        // The only payloads the live trace carries: 010100 and 010200.
        for (raw, action) in [(hex("010100"), 1), (hex("010200"), 2)] {
            let mut reader = PacketReader::new(&raw);
            let decoded = PlayerCommand::decode_payload(&mut reader).expect("decodable");
            reader.ensure_empty().expect("no trailing bytes");
            assert_eq!(
                decoded,
                PlayerCommand {
                    entity_id: 1,
                    action,
                    data: 0
                }
            );
            assert_eq!(decoded.encode_payload(), raw, "round-trips the trace bytes");
        }
    }

    #[test]
    fn set_health_roundtrips_the_s7_join_bytes() {
        // The S7 vanilla join of the saved Alice (`docs/PLAYERDATA.md` §1.3):
        // health 15.666667, food 17, saturation 0.
        let raw = hex("417aaaab1100000000");
        let mut reader = PacketReader::new(&raw);
        let decoded = SetHealth::decode_payload(&mut reader).expect("decodable");
        reader.ensure_empty().expect("no trailing bytes");
        assert_eq!(
            decoded,
            SetHealth {
                health: 15.666_667,
                food: 17,
                saturation: 0.0,
            }
        );
        assert_eq!(decoded.encode_payload(), raw, "round-trips the trace bytes");

        // A new player's 20/20/5.
        let fresh = SetHealth {
            health: 20.0,
            food: 20,
            saturation: 5.0,
        };
        let bytes = fresh.encode_payload();
        assert_eq!(bytes, hex("41a000001440a00000"));
        let mut reader = PacketReader::new(&bytes);
        assert_eq!(
            SetHealth::decode_payload(&mut reader).expect("decodable"),
            fresh
        );
    }

    #[test]
    fn punch_is_the_empty_payload() {
        // The live trace carries punch with zero bytes; anything else is an
        // error rather than a guess.
        let mut reader = PacketReader::new(&[]);
        let decoded = Punch::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, Punch);
        assert!(Punch.encode_payload().is_empty());
        let mut reader = PacketReader::new(&[0x00]);
        Punch::decode_payload(&mut reader).expect_err("a trailing byte is not punch");
    }

    /// The two `player_action` packets that broke (93,123,−22) in the vanilla
    /// S3 trace: the exact bytes, action values and the echoed sequence.
    #[test]
    fn player_action_roundtrips_the_s3_break() {
        let position = Position {
            x: 93,
            y: 123,
            z: -22,
        };
        let start = PlayerAction {
            action: PlayerAction::ACTION_START,
            position,
            face: 0x01,
            sequence: 1,
        };
        let finish = PlayerAction {
            action: PlayerAction::ACTION_FINISH,
            position,
            face: 0x01,
            sequence: 2,
        };
        crate::packets::test_support::check(
            start,
            "000000177ffffea07b0101",
            |packet| Ok(packet.encode_payload()),
            PlayerAction::decode_payload,
        );
        crate::packets::test_support::check(
            finish,
            "030000177ffffea07b0102",
            |packet| Ok(packet.encode_payload()),
            PlayerAction::decode_payload,
        );
        // The action constants match the recorded bytes.
        assert_eq!(start.action, 0x00, "start is 0x00 in the trace");
        assert_eq!(finish.action, 0x03, "finish is 0x03 in the trace");
    }

    /// The `block_update` and the two `block_changed_ack` payloads of the S3
    /// trace: `pos + state 0` and the echoed sequences.
    #[test]
    fn block_update_and_ack_roundtrip_the_s3_break() {
        crate::packets::test_support::check(
            BlockUpdate {
                position: Position {
                    x: 93,
                    y: 123,
                    z: -22,
                },
                state: 0,
            },
            "0000177ffffea07b00",
            |packet| Ok(packet.encode_payload()),
            BlockUpdate::decode_payload,
        );
        for (sequence, raw) in [(1, "01"), (2, "02")] {
            crate::packets::test_support::check(
                BlockChangedAck { sequence },
                raw,
                |packet| Ok(packet.encode_payload()),
                BlockChangedAck::decode_payload,
            );
        }
    }

    /// The `block_destruction` payloads of the 2026-11-14 traces: the entity
    /// id varint, the packed position and the stage byte, straight from the
    /// proxy capture (`docs/BLOCK-BREAKING.md` §1.3). `0xFF` is the reset;
    /// stage `00` starts a slow block (stone/dirt), stage `01` a fast one
    /// (leaves), and the packet id itself is 5 (`packets.json`).
    #[test]
    fn block_destruction_roundtrips_the_traced_stages() {
        let cases = [
            (
                51,
                Position {
                    x: 97,
                    y: 144,
                    z: -34,
                },
                0x01,
                "330000187ffffde09001",
            ),
            (
                826,
                Position {
                    x: 97,
                    y: 141,
                    z: -22,
                },
                0x01,
                "ba060000187ffffea08d01",
            ),
            (
                1069,
                Position {
                    x: 95,
                    y: 127,
                    z: -22,
                },
                0x00,
                "ad08000017fffffea07f00",
            ),
            (
                1034,
                Position {
                    x: 93,
                    y: 123,
                    z: -22,
                },
                0x00,
                "8a080000177ffffea07b00",
            ),
        ];
        for (entity_id, position, stage, raw) in cases {
            crate::packets::test_support::check(
                BlockDestruction {
                    entity_id,
                    position,
                    stage,
                },
                raw,
                |packet| Ok(packet.encode_payload()),
                BlockDestruction::decode_payload,
            );
        }
        // The reset byte and the id are the ones the traces and the packet
        // report carry.
        assert_eq!(BlockDestruction::RESET, 0xFF, "0xFF clears the cracks");
        assert_eq!(BlockDestruction::ID, 0x05, "block_destruction is id 5");
    }

    /// The block-break `level_event` the S3 observer received after the
    /// grass block (93,123,−22) broke: event `2001`, `data 9` = the removed
    /// state id (`grass_block`, `reports/blocks.json`), trailing `0x00`.
    #[test]
    fn level_event_roundtrips_the_s3_break() {
        crate::packets::test_support::check(
            LevelEvent {
                event: LevelEvent::EVENT_BLOCK_BREAK,
                position: Position {
                    x: 93,
                    y: 123,
                    z: -22,
                },
                data: 9,
                trailing: LevelEvent::TRACED_TRAILING,
            },
            "000007d10000177ffffea07b0000000900",
            |packet| Ok(packet.encode_payload()),
            LevelEvent::decode_payload,
        );
    }

    /// The block-break `level_event` the S9 observer received twice after
    /// the creative digger broke dark oak leaves at (96,143,−30): `data 450`
    /// = the removed state id (`dark_oak_leaves`, `reports/blocks.json`).
    #[test]
    fn level_event_roundtrips_the_s9_break() {
        crate::packets::test_support::check(
            LevelEvent {
                event: LevelEvent::EVENT_BLOCK_BREAK,
                position: Position {
                    x: 96,
                    y: 143,
                    z: -30,
                },
                data: 450,
                trailing: LevelEvent::TRACED_TRAILING,
            },
            "000007d10000183ffffe208f000001c200",
            |packet| Ok(packet.encode_payload()),
            LevelEvent::decode_payload,
        );
    }

    /// The S14 instant-break and water-block payloads, straight from the
    /// 2026-11-17 vanilla trace (`docs/BLOCK-BREAKING.md` §1.12): the digger's
    /// `block_update` of the replacement (air for short grass, the water
    /// source for seagrass/kelp/waterlogged), the observer's break
    /// `level_event` (data = the broken state id), and the acks that echo the
    /// client's own sequences. The instant-break sequence is start-only, so the
    /// acks are the start sequences (1, 25, 39, 40).
    #[test]
    fn s14_instant_break_packets_roundtrip_the_trace() {
        for (x, y, z, state, raw) in [
            (96, 201, -34, 0, "0000183ffffde0c900"),
            (108, 201, -34, 89, "00001b3ffffde0c959"),
            (116, 201, -34, 89, "00001d3ffffde0c959"),
            (120, 201, -34, 89, "00001e3ffffde0c959"),
        ] {
            crate::packets::test_support::check(
                BlockUpdate {
                    position: Position { x, y, z },
                    state,
                },
                raw,
                |packet| Ok(packet.encode_payload()),
                BlockUpdate::decode_payload,
            );
        }
        for (x, y, z, data, raw) in [
            (96, 201, -34, 2363, "000007d10000183ffffde0c90000093b00"),
            (108, 201, -34, 2370, "000007d100001b3ffffde0c90000094200"),
            (112, 201, -34, 2372, "000007d100001c3ffffde0c90000094400"),
            (112, 202, -34, 2371, "000007d100001c3ffffde0ca0000094300"),
            (116, 201, -34, 18421, "000007d100001d3ffffde0c9000047f500"),
            (116, 202, -34, 18445, "000007d100001d3ffffde0ca0000480d00"),
            (116, 203, -34, 18424, "000007d100001d3ffffde0cb000047f800"),
            (120, 201, -34, 15179, "000007d100001e3ffffde0c900003b4b00"),
            (124, 201, -34, 5533, "000007d100001f3ffffde0c90000159d00"),
        ] {
            crate::packets::test_support::check(
                LevelEvent {
                    event: LevelEvent::EVENT_BLOCK_BREAK,
                    position: Position { x, y, z },
                    data,
                    trailing: LevelEvent::TRACED_TRAILING,
                },
                raw,
                |packet| Ok(packet.encode_payload()),
                LevelEvent::decode_payload,
            );
        }
        for (sequence, raw) in [(1, "01"), (25, "19"), (39, "27"), (40, "28")] {
            crate::packets::test_support::check(
                BlockChangedAck { sequence },
                raw,
                |packet| Ok(packet.encode_payload()),
                BlockChangedAck::decode_payload,
            );
        }
    }

    #[test]
    fn disconnect_play_matches_the_s15_duplicate_login_bytes() {
        let packet = DisconnectPlay {
            reason: DisconnectPlay::DUPLICATE_LOGIN.to_owned(),
        };
        let encoded = packet.encode_payload().expect("encodable");
        // The recorded vanilla payload of `2026-11-17_s15-vanilla-r2`.
        let recorded = concat!(
            "0a0800097472616e736c61746500266d756c7469706c617965722e646973636f6e6e656374",
            "2e6475706c69636174655f6c6f67696e00"
        );
        let expected: Vec<u8> = (0..recorded.len() / 2)
            .map(|i| u8::from_str_radix(&recorded[i * 2..i * 2 + 2], 16).expect("hex"))
            .collect();
        assert_eq!(encoded, expected);

        // The same bytes decoded as the network NBT form name the key.
        let (tag, consumed) = NbtTag::decode_network(&encoded).expect("network NBT");
        assert_eq!(consumed, encoded.len());
        let NbtTag::Compound(map) = tag else {
            panic!("a compound root");
        };
        assert_eq!(
            map.get("translate"),
            Some(&NbtTag::String(DisconnectPlay::DUPLICATE_LOGIN.to_owned()))
        );
    }

    #[test]
    fn synchronize_position_roundtrips_a_teleport() {
        let packet = SynchronizePlayerPosition {
            teleport_id: 1,
            x: 8.5,
            y: 64.0,
            z: 8.5,
            velocity_x: 0.0,
            velocity_y: 0.0,
            velocity_z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
        };
        let frame = packet.to_frame().expect("encodable");
        let mut reader = decoded!(frame, SynchronizePlayerPosition::ID);

        let decoded = SynchronizePlayerPosition::decode_payload(&mut reader).expect("decodable");
        assert_eq!(decoded, packet);
        reader.ensure_empty().expect("no trailing bytes");
    }

    /// The S24a vanilla `set_creative_mode_slot` payloads
    /// (`E:\MineRust-data\traces\s24a_decode.txt`): slot, count, item id, no
    /// components. Decoding the recorded bytes and encoding them back must give
    /// the same bytes.
    #[test]
    fn s24a_creative_slots_roundtrip_the_trace() {
        for (raw, slot, item, count) in [
            ("002440010000", 36, 1, 64),
            ("0025403f0000", 37, 63, 64),
            ("002640a3010000", 38, 163, 64),
            ("002740d5020000", 39, 341, 64),
            ("00284083040000", 40, 515, 64),
            ("002940e7010000", 41, 231, 64),
        ] {
            let raw = hex(raw);
            let mut reader = PacketReader::new(&raw);
            let packet =
                SetCreativeModeSlot::decode_payload(&mut reader).expect("decodable trace payload");
            reader.ensure_empty().expect("the stack ends the payload");
            assert_eq!(packet.slot, slot);
            assert_eq!(packet.item.item, item);
            assert_eq!(packet.item.count, count);
            assert!(!packet.item.is_empty());
            assert_eq!(
                packet.encode_payload(),
                raw,
                "re-encoding keeps the component bytes verbatim"
            );
        }
    }

    /// An empty creative slot is `count 0` and nothing else: the codec must not
    /// invent an item id for it.
    #[test]
    fn an_empty_creative_slot_is_the_count_only() {
        let raw = hex("000000");
        let mut reader = PacketReader::new(&raw);
        let packet = SetCreativeModeSlot::decode_payload(&mut reader).expect("decodable");
        assert_eq!(packet.slot, 0);
        assert!(packet.item.is_empty());
        assert_eq!(packet.encode_payload(), raw);
    }

    /// The S24a `set_carried_item` payloads: a big-endian short, nothing else.
    #[test]
    fn s24a_carried_item_roundtrips_the_trace() {
        for (raw, slot) in [("0001", 1), ("0008", 8), ("0000", 0)] {
            let raw = hex(raw);
            let mut reader = PacketReader::new(&raw);
            let packet = SetCarriedItem::decode_payload(&mut reader).expect("decodable");
            reader.ensure_empty().expect("no trailing bytes");
            assert_eq!(packet.slot, slot);
            assert_eq!(packet.encode_payload(), raw);
        }
    }

    /// The S17 `pick_item_from_block` payload: packed position + `include_data`.
    #[test]
    fn s17_pick_item_from_block_roundtrips_the_trace() {
        let raw = hex("0000143ffffd80c800");
        let mut reader = PacketReader::new(&raw);
        let packet = PickItemFromBlock::decode_payload(&mut reader).expect("decodable");
        reader.ensure_empty().expect("no trailing bytes");
        assert_eq!(
            packet.position,
            Position {
                x: 80,
                y: 200,
                z: -40
            }
        );
        assert!(!packet.include_data);
        assert_eq!(packet.encode_payload(), raw);
    }

    /// The S17 and S24b `use_item_on` payloads, byte for byte: hand 0, the
    /// packed position, the face, the three cursor floats, the two flags and
    /// the sequence.
    #[test]
    fn use_item_on_roundtrips_the_traces() {
        for (raw, x, y, z, face, sequence) in [
            (
                "000000143ffffec0ca023f0000003eed009100000000000001",
                80,
                202,
                -20,
                2,
                1,
            ),
            (
                "00000015bffffec0ca023f0000003eed009100000000000002",
                86,
                202,
                -20,
                2,
                2,
            ),
            (
                "000000143ffffd80ca053f8000003f1eb8523f000000000001",
                80,
                202,
                -40,
                5,
                1,
            ),
            (
                "0000002d3ffffd70c8013f0000003f8000003f000000000005",
                180,
                200,
                -41,
                1,
                5,
            ),
            (
                "0000002abffffd80c9013f0000003f8000003efe75b2000006",
                170,
                201,
                -40,
                1,
                6,
            ),
            (
                "00000039bffffd80c9013f0000003f8000003efe75b2000007",
                230,
                201,
                -40,
                1,
                7,
            ),
        ] {
            let raw_bytes = hex(raw);
            let mut reader = PacketReader::new(&raw_bytes);
            let packet = UseItemOn::decode_payload(&mut reader).expect("decodable trace payload");
            reader.ensure_empty().expect("no trailing bytes");
            assert_eq!(packet.hand, 0);
            assert_eq!(packet.position, Position { x, y, z });
            assert_eq!(packet.face, face);
            assert!(!packet.inside_block);
            assert!(!packet.world_border);
            assert_eq!(packet.sequence, sequence);
            assert_eq!(packet.encode_payload(), raw_bytes);
        }
    }

    /// The S24a join `container_set_content`: window 0, state 1, 46 empty slots,
    /// an empty cursor — 50 bytes.
    #[test]
    fn s24a_container_set_content_matches_the_trace() {
        let raw = hex("00012e0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000");
        let packet = ContainerSetContent {
            window: 0,
            state_id: 1,
            items: vec![ItemStack::empty(); 46],
            carried: ItemStack::empty(),
        };
        assert_eq!(packet.encode_payload(), raw);
        assert_eq!(raw.len(), 50);
    }

    /// The S17 creative-pick answers: `set_held_slot` and `container_set_slot`.
    #[test]
    fn s17_pick_answers_match_the_trace() {
        for (raw, slot) in [("00", 0), ("04", 4), ("08", 8)] {
            assert_eq!(SetHeldSlot { slot }.encode_payload(), hex(raw));
        }
        for (raw, state, slot, item) in [
            ("0002002401010000", 2, 36, 1),
            ("0003002501370000", 3, 37, 55),
            ("0005002701e7010000", 5, 39, 231),
        ] {
            let packet = ContainerSetSlot {
                window: 0,
                state_id: state,
                slot,
                item: ItemStack::new(item, 1),
            };
            assert_eq!(packet.encode_payload(), hex(raw));
        }
    }

    /// A stack with components keeps them verbatim on the way back out: the
    /// codec neither parses nor rewrites component bytes.
    #[test]
    fn a_stack_keeps_its_component_bytes() {
        let mut writer = PacketWriter::new();
        ItemStack::new(1, 1).encode(&mut writer);
        let mut bytes = writer.into_bytes();
        assert_eq!(bytes, vec![1, 1, 0, 0]);
        // Give the stack a component tail: one added component, one removed.
        bytes.pop();
        bytes.pop();
        bytes.extend_from_slice(&[1, 0x2a, 0x05, 7]);
        let mut reader = PacketReader::new(&bytes);
        let stack = ItemStack::decode_tail(&mut reader, MAX_ITEM_COMPONENTS).expect("decodable");
        reader.ensure_empty().expect("consumed");
        assert_eq!(stack.components, vec![1, 0x2a, 0x05, 7]);
        let mut writer = PacketWriter::new();
        stack.encode(&mut writer);
        assert_eq!(writer.into_bytes(), bytes);
    }
}

#[cfg(test)]
mod chat_tests {
    //! Byte tests for the step-25 chat, ability and time packets: every
    //! expectation is a payload captured from vanilla 26.3 (traces of step
    //! 25.0), and the only tolerated difference is NBT compound key order.

    use super::*;

    /// Parses a hex dump with spaces or newlines.
    fn hex(text: &str) -> Vec<u8> {
        text.split_whitespace()
            .map(|byte| u8::from_str_radix(byte, 16).expect("hex byte"))
            .collect()
    }

    /// The serverbound chat of the S19 trace (39 bytes).
    // SOURCE: 2026-11-18_s19s20-vanilla-step25-r1, conn-1 seq 62417.
    const S19_CHAT: &str = "\
        10 73 31 39 20 68 65 6c 6c 6f 20 66 72 6f 6d 20 41 00 00 01 a1 ef fe 1a a0 \
        77 0f 7f 60 99 73 65 85 00 00 00 00 00 01";

    /// Everything of the S19 `player_chat` before the sender-name component.
    // SOURCE: 2026-11-18_s19s20-vanilla-step25-r1, conn-1 seq 62427.
    const S19_PLAYER_CHAT_PREFIX: &str = "\
        00 14 af be 82 83 6c 32 06 8b 29 73 72 6b 87 9d e6 00 00 10 73 31 39 20 68 \
        65 6c 6c 6f 20 66 72 6f 6d 20 41 00 00 01 a1 ef fe 1a ab 00 00 00 00 00 00 \
        00 00 00 00 00 01";

    #[test]
    fn the_s19_chat_decodes_to_its_text() {
        let bytes = hex(S19_CHAT);
        assert_eq!(bytes.len(), 39);
        let mut reader = PacketReader::new(&bytes);
        let packet = ChatMessage::decode_payload(&mut reader).expect("decodable");
        assert_eq!(packet.message, "s19 hello from A");
        // The tail (timestamp, salt, signature and acknowledgement) is not
        // read, so the reader is not empty; a decode keeps working.
        assert!(!reader.is_empty());
    }

    #[test]
    fn the_s19_player_chat_matches_the_trace() {
        let sender = hex("14 af be 82 83 6c 32 06 8b 29 73 72 6b 87 9d e6");
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&sender);
        let line = PlayerChat {
            global_index: 0,
            sender: uuid,
            content: "s19 hello from A".to_owned(),
            timestamp: 0x0000_01a1_effe_1aab,
            sender_name: PlayerChat::name_component("ChatA", uuid),
        };
        let payload = line.encode_payload().expect("encodable");
        let prefix = hex(S19_PLAYER_CHAT_PREFIX);
        assert_eq!(payload.len(), 258, "the traced length");
        assert_eq!(&payload[..prefix.len()], &prefix[..], "the traced prefix");
        // Root compound end, the hover compound end and the absent target
        // name close the packet.
        // Our compound keys are sorted, so the hover compound closes before
        // `insertion`/`text` while vanilla closes it last; the packet ends
        // with the root compound end and the absent target name. The traced
        // bytes carry one more `00` there for that reason.
        assert_eq!(&payload[256..], &[0x00, 0x00]);
    }

    #[test]
    fn the_sender_name_component_roundtrips() {
        let uuid = protocol_uuid();
        let component = PlayerChat::name_component("ChatA", uuid);
        let mut writer = PacketWriter::new();
        writer.write_nbt(&component).expect("encodable");
        let bytes = writer.into_bytes();
        let mut reader = PacketReader::new(&bytes);
        let decoded = reader.read_nbt().expect("decodable");
        reader.ensure_empty().expect("the whole component");
        assert_eq!(decoded, component);
    }

    /// The offline UUID of `ChatA` as the traces carry it.
    fn protocol_uuid() -> Uuid {
        crate::offline_uuid("ChatA")
    }

    #[test]
    fn the_s20_join_line_has_the_traced_length() {
        let uuid = crate::offline_uuid("JoinB");
        let line = SystemChat::join_leave("multiplayer.player.joined", "JoinB", uuid);
        let payload = line.encode_payload().expect("encodable");
        // SOURCE: 2026-11-18_s20-vanilla-step25-r4, conn-1 seq 49174: the
        // joined line is 270 bytes with its trailing overlay flag.
        assert_eq!(payload.len(), 270);
        assert_eq!(*payload.last().expect("non-empty"), 0);
        let mut reader = PacketReader::new(&payload[..payload.len() - 1]);
        let decoded = reader.read_nbt().expect("decodable");
        reader.ensure_empty().expect("component then the flag");
        assert_eq!(decoded, line.component);
    }

    #[test]
    fn player_abilities_carry_the_traced_mode_bytes() {
        // SOURCE: 2026-11-18_s21-l2-vanilla-step25-r2: creative `0d ...`,
        // spectator `07 ...`; survival and adventure `00 ...`.
        let speeds = "3d 4c cc cd 3d cc cc cd";
        for (mode, flags) in [(1, "0d"), (3, "07"), (0, "00"), (2, "00")] {
            let bytes = PlayerAbilities::for_mode(mode).encode_payload();
            assert_eq!(bytes, hex(&format!("{flags} {speeds}")), "mode {mode}");
        }
    }

    #[test]
    fn set_time_decodes_the_join_and_the_time_set_packets() {
        // SOURCE: 2026-11-18_s21-l2-vanilla-step25-r2, conn-1 seq 60 (join,
        // two clocks) and seq 106749 (`/time set day`, one clock).
        let join = hex(
            "00 00 00 00 00 00 0b b5 02 01 b5 17 00 00 00 00 3f 80 00 00 \
             00 b5 17 00 00 00 00 3f 80 00 00",
        );
        assert_eq!(join.len(), 31);
        let mut reader = PacketReader::new(&join);
        let packet = SetTime::decode_payload(&mut reader).expect("decodable");
        reader.ensure_empty().expect("the whole packet");
        assert_eq!(packet.game_time, 0x0000_0000_0000_0bb5);
        assert_eq!(
            packet.clocks,
            vec![
                ClockUpdate {
                    id: 1,
                    total_ticks: 2997,
                    partial_tick: 0.0,
                    rate: 1.0,
                },
                ClockUpdate {
                    id: 0,
                    total_ticks: 2997,
                    partial_tick: 0.0,
                    rate: 1.0,
                },
            ]
        );
        assert_eq!(
            SetTime {
                game_time: packet.game_time,
                clocks: packet.clocks.clone(),
            }
            .encode_payload(),
            join
        );

        let day = hex("00 00 00 00 00 00 13 41 01 00 a8 c3 01 00 00 00 00 3f 80 00 00");
        assert_eq!(day.len(), 21);
        let mut reader = PacketReader::new(&day);
        let packet = SetTime::decode_payload(&mut reader).expect("decodable");
        reader.ensure_empty().expect("the whole packet");
        assert_eq!(
            packet,
            SetTime {
                game_time: 0x0000_0000_0000_1341,
                clocks: vec![ClockUpdate {
                    id: 0,
                    total_ticks: 25_000,
                    partial_tick: 0.0,
                    rate: 1.0,
                }],
            }
        );
        assert_eq!(packet.encode_payload(), day);
    }
}
