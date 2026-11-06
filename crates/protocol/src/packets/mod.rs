//! Packet definitions for the handshake and status states.
//!
//! Packet ids and field layouts follow the public protocol documentation
//! (<https://minecraft.wiki/wiki/Java_Edition_protocol/Server_List_Ping>).

mod handshake;
mod status;

pub use handshake::{Handshake, NextState};
pub use status::{PingRequest, PongResponse, StatusRequest, StatusResponse};
