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
use crate::packet_id::{PacketKey, ProtocolState};
use crate::packets::login::{ProfileProperty, MAX_PROPERTY_NAME_CHARS, MAX_SIGNATURE_CHARS};

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
/// The payload is two big-endian `i32` (8 bytes): the layout the vanilla
/// 26.3 walking traces show (`window-p4/p5.jsonl`, M4.4 B) — not packed
/// coordinates and not VarInts. SOURCE: parity trace; ids from
/// `reports/packets.json` (protocol id 38).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForgetLevelChunk {
    /// Chunk X to drop.
    pub chunk_x: i32,
    /// Chunk Z to drop.
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

    /// Encodes the payload (packet id excluded): two big-endian `i32`.
    pub fn encode_payload(&self) -> Vec<u8> {
        let mut writer = PacketWriter::new();
        writer.write_i32(self.chunk_x);
        writer.write_i32(self.chunk_z);
        writer.into_bytes()
    }

    /// Decodes the payload; the packet id must already be consumed.
    pub fn decode_payload(reader: &mut PacketReader<'_>) -> Result<Self, CodecError> {
        Ok(Self {
            chunk_x: reader.read_i32()?,
            chunk_z: reader.read_i32()?,
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

    /// `forget_level_chunk` encodes to two big-endian `i32`, byte for byte
    /// the payloads of the vanilla 26.3 walking trace `window-p4.jsonl`
    /// (seq 16000 and 16022); negative coordinates round-trip too.
    #[test]
    fn forget_level_chunk_matches_the_traced_payloads() {
        // seq 16000: chunk (-8, -4) → ff ff ff f8 ff ff ff fc.
        let traced_negative = [0xFF, 0xFF, 0xFF, 0xF8, 0xFF, 0xFF, 0xFF, 0xFC];
        // seq 16022: chunk (9, 1) → 00 00 00 09 00 00 00 01.
        let traced_positive = [0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00, 0x01];
        let cases = [
            (
                ForgetLevelChunk {
                    chunk_x: -8,
                    chunk_z: -4,
                },
                traced_negative,
            ),
            (
                ForgetLevelChunk {
                    chunk_x: 9,
                    chunk_z: 1,
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
}
