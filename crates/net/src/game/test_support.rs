//! Shared fixtures of the game-thread scenario tests (M4.3 part B).

use std::sync::Arc;

use protocol::Position;
use tokio::sync::mpsc;

use super::messages::PlayerConnection;
use super::{
    KickFlag, MoveKind, MoveSlot, MoveState, PlayerId, PlayerJoin, ServerMessage, OUTBOUND_CAPACITY,
};

/// Tile side in chunks of every scenario here.
pub const SIDE: i32 = 2;

/// A join registration plus the pieces a test needs to drive it: the movement
/// slot the connection writes and the connection's side of the outbound queue.
pub type JoinFixture = (PlayerJoin, Arc<MoveSlot>, mpsc::Receiver<ServerMessage>);

/// A join registration at a block, with the player's movement slot and the
/// connection's side of the outbound queue.
pub fn join_at(id: PlayerId, spawn: (i32, i32, i32)) -> JoinFixture {
    let (outbound, receiver) = mpsc::channel(OUTBOUND_CAPACITY);
    let slot = Arc::new(MoveSlot::default());
    let join = PlayerJoin {
        id,
        name: format!("player-{id}"),
        view_distance: 10,
        spawn: Position {
            x: spawn.0,
            y: spawn.1,
            z: spawn.2,
        },
        connection: PlayerConnection {
            outbound,
            kick: Arc::new(KickFlag::default()),
            last_move: Arc::clone(&slot),
        },
    };
    (join, slot, receiver)
}

/// Payload of `move_player_pos`: three big-endian doubles and the flags byte.
pub fn pos_move(x: f64, y: f64, z: f64) -> MoveState {
    let mut payload = Vec::with_capacity(25);
    for value in [x, y, z] {
        payload.extend_from_slice(&value.to_be_bytes());
    }
    payload.push(0x01);
    MoveState {
        kind: MoveKind::Pos,
        payload,
    }
}

/// Payload of `move_player_pos_rot`, the shape the replay playback rebuilds a
/// recorded move into (all axes absolute, flags verbatim).
pub fn pos_rot_move(x: f64, y: f64, z: f64, yaw: f32, pitch: f32, flags: u8) -> MoveState {
    let mut payload = Vec::with_capacity(33);
    for value in [x, y, z] {
        payload.extend_from_slice(&value.to_be_bytes());
    }
    payload.extend_from_slice(&yaw.to_be_bytes());
    payload.extend_from_slice(&pitch.to_be_bytes());
    payload.push(flags);
    MoveState {
        kind: MoveKind::PosRot,
        payload,
    }
}
