//! Packet definitions for the handshake, status, login, configuration and
//! play states.
//!
//! Packet ids and field layouts follow the public protocol documentation
//! (<https://minecraft.wiki/wiki/Java_Edition_protocol>).

mod configuration;
mod handshake;
mod login;
mod play;
mod status;

pub use configuration::{
    ClientInformation, FinishAck, FinishConfiguration, RegistryData, RegistryEntry, RegistryTag,
    TagRegistry, UpdateTags,
};
pub use handshake::{Handshake, NextState};
pub use login::{
    offline_uuid, DisconnectLogin, GameProfile, LoginAcknowledged, LoginStart, LoginSuccess,
    ProfileProperty, SetCompression,
};
pub use play::{
    ConfirmTeleportation, DeathLocation, GameEvent, KeepAlive, LoginPlay, PlayerInfoEntry,
    PlayerInfoUpdate, SetCenterChunk, SetDefaultSpawnPosition, SynchronizePlayerPosition,
};
pub use status::{PingRequest, PongResponse, StatusRequest, StatusResponse};
