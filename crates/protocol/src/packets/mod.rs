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

mod configuration;
mod handshake;
mod login;
mod play;
mod status;

pub use configuration::{
    ClientInformation, FinishAck, FinishConfiguration, RegistryData, RegistryEntry, RegistryTag,
    TagRegistry, UpdateTags, CONFIGURATION_PLUGIN_MESSAGE,
};
pub use handshake::{Handshake, NextState};
pub use login::{
    offline_uuid, DisconnectLogin, GameProfile, LoginAcknowledged, LoginStart, LoginSuccess,
    ProfileProperty, SetCompression,
};
pub use play::{
    ConfirmTeleportation, DeathLocation, GameEvent, KeepAlive, LoginPlay, PlayerInfoEntry,
    PlayerInfoUpdate, SetCenterChunk, SetDefaultSpawnPosition, SynchronizePlayerPosition,
    MOVE_PLAYER_POS, MOVE_PLAYER_POS_ROT, MOVE_PLAYER_ROT, MOVE_PLAYER_STATUS_ONLY,
    PLAY_PLUGIN_MESSAGE,
};
pub use status::{PingRequest, PongResponse, StatusRequest, StatusResponse};

use crate::packet_id::PacketKey;

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
        // play, clientbound
        LoginPlay::KEY,
        SetDefaultSpawnPosition::KEY,
        PlayerInfoUpdate::KEY,
        SetCenterChunk::KEY,
        GameEvent::KEY,
        SynchronizePlayerPosition::KEY,
        KeepAlive::KEY,
        // play, serverbound
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
