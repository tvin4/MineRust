//! Packet definitions for the handshake, status, login, configuration and
//! play states.
//!
//! Packet ids and field layouts follow the public protocol documentation
//! (<https://minecraft.wiki/wiki/Java_Edition_protocol>).
//!
//! SOURCE: wiki, unverified — every compiled default id and field layout
//! starts here as a wiki value; it becomes verified only when the packets
//! report of the game (`reports/packets.json`) or a parity test confirms it
//! (see `docs/SOURCES.md`). Individual unknowns carry their own
//! `// SOURCE: wiki, unverified` markers.

mod chunk;
mod chunk_payload;
mod configuration;
mod entity;
mod entity_meta;
mod handshake;
mod login;
mod movement;
mod packet_impl;
mod play;
mod status;
#[cfg(test)]
mod test_support;

pub use chunk::{ChunkBatchFinished, ChunkBatchReceived, ChunkBatchStart, LevelChunkWithLight};
pub use chunk_payload::{
    set_bits, BlockEntity, ChunkSection, HeightmapRecord, LightData, PaletteContainer, BIOME_CELLS,
    BLOCK_CELLS, LIGHT_ARRAY_BYTES,
};
pub use configuration::{
    ClientInformation, FinishAck, FinishConfiguration, RegistryData, RegistryEntry, RegistryTag,
    TagRegistry, UpdateTags, CONFIGURATION_CLIENTBOUND_PLUGIN_MESSAGE,
    CONFIGURATION_PLUGIN_MESSAGE, SELECT_KNOWN_PACKS, SELECT_KNOWN_PACKS_SERVERBOUND,
    UPDATE_ENABLED_FEATURES,
};
pub use entity::{AddEntity, PlayerInfoRemove, RemoveEntities};
pub use entity_meta::{
    AttributeEntry, AttributeModifier, EntityPositionSync, SetEntityData, UpdateAttributes,
};
pub use handshake::{Handshake, NextState};
pub use login::{
    offline_uuid, DisconnectLogin, GameProfile, LoginAcknowledged, LoginStart, LoginSuccess,
    ProfileProperty, SetCompression,
};
pub use movement::{MoveEntityPos, MoveEntityPosRot, MoveEntityRot, MoveHeader, RotateHead};
pub use play::{
    hashed_seed, ConfirmTeleportation, DeathLocation, ForgetLevelChunk, GameEvent, KeepAlive,
    LoginPlay, PlayerInfoEntry, PlayerInfoUpdate, SetCenterChunk, SetDefaultSpawnPosition,
    SynchronizePlayerPosition, MOVE_PLAYER_POS, MOVE_PLAYER_POS_ROT, MOVE_PLAYER_ROT,
    MOVE_PLAYER_STATUS_ONLY, PLAY_PLUGIN_MESSAGE,
};
pub use status::{PingRequest, PongResponse, StatusRequest, StatusResponse};

use crate::codec::{CodecError, PacketReader};
use crate::packet_id::PacketKey;

/// Reads a non-negative VarInt count.
pub(crate) fn read_count(reader: &mut PacketReader<'_>) -> Result<usize, CodecError> {
    let count = reader.read_varint()?;
    usize::try_from(count).map_err(|_| CodecError::NegativeLength(count))
}

/// Every packet id the server sends or receives, by key.
///
/// The list is the one place that decides which numeric ids the server
/// resolves at runtime; tests compare each key's compiled default with the
/// data generator's packets report of the user's game copy, and the server
/// startup verifies that a configured report contains them all.
pub fn used_packet_keys() -> &'static [PacketKey] {
    &[
        // handshake
        Handshake::KEY,
        // status
        StatusRequest::KEY,
        StatusResponse::KEY,
        PingRequest::KEY,
        PongResponse::KEY,
        // login
        LoginStart::KEY,
        SetCompression::KEY,
        LoginSuccess::KEY,
        LoginAcknowledged::KEY,
        DisconnectLogin::KEY,
        // configuration
        RegistryData::KEY,
        UpdateTags::KEY,
        FinishConfiguration::KEY,
        FinishAck::KEY,
        ClientInformation::KEY,
        CONFIGURATION_PLUGIN_MESSAGE,
        CONFIGURATION_CLIENTBOUND_PLUGIN_MESSAGE,
        SELECT_KNOWN_PACKS,
        SELECT_KNOWN_PACKS_SERVERBOUND,
        UPDATE_ENABLED_FEATURES,
        // play, clientbound
        LoginPlay::KEY,
        SetDefaultSpawnPosition::KEY,
        PlayerInfoUpdate::KEY,
        SetCenterChunk::KEY,
        ForgetLevelChunk::KEY,
        GameEvent::KEY,
        SynchronizePlayerPosition::KEY,
        KeepAlive::KEY,
        LevelChunkWithLight::KEY,
        ChunkBatchStart::KEY,
        ChunkBatchFinished::KEY,
        // play, clientbound entity stream (M4.5)
        AddEntity::KEY,
        RemoveEntities::KEY,
        PlayerInfoRemove::KEY,
        MoveEntityPos::KEY,
        MoveEntityPosRot::KEY,
        MoveEntityRot::KEY,
        RotateHead::KEY,
        SetEntityData::KEY,
        UpdateAttributes::KEY,
        EntityPositionSync::KEY,
        // play, serverbound
        ChunkBatchReceived::KEY,
        ConfirmTeleportation::KEY,
        KeepAlive::SERVERBOUND_KEY,
        ClientInformation::PLAY_KEY,
        PLAY_PLUGIN_MESSAGE,
        MOVE_PLAYER_POS,
        MOVE_PLAYER_POS_ROT,
        MOVE_PLAYER_ROT,
        MOVE_PLAYER_STATUS_ONLY,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet_id::ProtocolState;

    #[test]
    fn used_keys_are_unique_in_state_direction_and_name() {
        let keys = used_packet_keys();
        assert!(!keys.is_empty());
        let mut seen = std::collections::HashSet::new();
        for key in keys {
            let id = (key.state, key.direction, key.name);
            assert!(seen.insert(id), "duplicate key: {}", key.describe());
        }
    }

    #[test]
    fn used_keys_cover_every_state_and_both_directions() {
        let mut states = [false; ProtocolState::COUNT];
        let mut clientbound = false;
        let mut serverbound = false;
        for key in used_packet_keys() {
            states[key.state.index()] = true;
            match key.direction {
                crate::packet_id::Direction::Clientbound => clientbound = true,
                crate::packet_id::Direction::Serverbound => serverbound = true,
            }
        }
        assert!(states.iter().all(|seen| *seen));
        assert!(clientbound && serverbound);
    }

    #[test]
    fn used_keys_carry_the_compiled_default_ids() {
        for key in used_packet_keys() {
            // Empty keys would silently drop packets when no report is
            // loaded; the default id is the id the server must send.
            assert_ne!(key.default_id, u32::MAX, "{}", key.describe());
            assert_eq!(key.default_id, key.default_id & 0xFF, "{}", key.describe());
        }
    }
}
