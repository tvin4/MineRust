//! Decoding of the movement packets, done in the tick (ARCH-M4 §0, M4.2).
//!
//! The connection stores the newest movement packet as its
//! [`MoveKind`](super::MoveKind) plus the raw payload; the game thread parses
//! those bytes here, once per tick, and applies them to the player
//! ([`Player::apply_move`](super::Player::apply_move)). No movement value is
//! interpreted on the network task.
//!
//! Field layouts follow the public protocol documentation: X/Y/Z are three
//! big-endian `f64` (Y is the feet), yaw/pitch are two `f32` and the trailing
//! byte is the on-ground flag. The formats match what the parity walk tool
//! writes for the live client (`crates/parity/src/walk.rs`).
// SOURCE: wiki, unverified — layouts are documented and match the parity
// walk payloads; only a live client proves the server side.

use protocol::{CodecError, PacketReader};

use super::messages::MoveState;

/// The values one movement packet carries; an absent axis is not touched.
///
/// [`MoveKind::Pos`](super::MoveKind::Pos) sets the position, `Rot` the
/// angles, `PosRot` both and `StatusOnly` only the on-ground byte, so the
/// tick can overlay them on the player's current state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MoveUpdate {
    /// Absolute X, when the packet carries a position.
    pub x: Option<f64>,
    /// Absolute Y (feet), when the packet carries a position.
    pub y: Option<f64>,
    /// Absolute Z, when the packet carries a position.
    pub z: Option<f64>,
    /// Absolute yaw in degrees, when the packet carries rotation.
    pub yaw: Option<f32>,
    /// Absolute pitch in degrees, when the packet carries rotation.
    pub pitch: Option<f32>,
    /// Whether the client claims to stand on the ground.
    pub on_ground: bool,
}

impl MoveState {
    /// Parses the payload according to the packet kind.
    ///
    /// An incomplete or over-long payload is an error rather than a guess:
    /// the tick logs it and drops the movement (the connection never
    /// validated the length).
    pub fn decode(&self) -> Result<MoveUpdate, CodecError> {
        let mut reader = PacketReader::new(&self.payload);
        let update = match self.kind {
            super::messages::MoveKind::Pos => MoveUpdate {
                x: Some(reader.read_f64()?),
                y: Some(reader.read_f64()?),
                z: Some(reader.read_f64()?),
                yaw: None,
                pitch: None,
                on_ground: reader.read_bool()?,
            },
            super::messages::MoveKind::PosRot => MoveUpdate {
                x: Some(reader.read_f64()?),
                y: Some(reader.read_f64()?),
                z: Some(reader.read_f64()?),
                yaw: Some(reader.read_f32()?),
                pitch: Some(reader.read_f32()?),
                on_ground: reader.read_bool()?,
            },
            super::messages::MoveKind::Rot => MoveUpdate {
                x: None,
                y: None,
                z: None,
                yaw: Some(reader.read_f32()?),
                pitch: Some(reader.read_f32()?),
                on_ground: reader.read_bool()?,
            },
            super::messages::MoveKind::StatusOnly => MoveUpdate {
                x: None,
                y: None,
                z: None,
                yaw: None,
                pitch: None,
                on_ground: reader.read_bool()?,
            },
        };
        reader.ensure_empty()?;
        Ok(update)
    }
}

#[cfg(test)]
mod tests {
    use super::super::messages::MoveKind;
    use super::*;

    /// Encodes a payload of big-endian doubles, floats and a trailing byte.
    fn payload(f64s: &[f64], f32s: &[f32], on_ground: bool) -> Vec<u8> {
        let mut out = Vec::new();
        for value in f64s {
            out.extend_from_slice(&value.to_be_bytes());
        }
        for value in f32s {
            out.extend_from_slice(&value.to_be_bytes());
        }
        out.push(u8::from(on_ground));
        out
    }

    #[test]
    fn a_position_packet_carries_the_position_and_the_flag() {
        let state = MoveState {
            kind: MoveKind::Pos,
            payload: payload(&[1.5, 64.0, -2.25], &[], true),
        };
        let update = state.decode().expect("a valid pos payload");
        assert_eq!(update.x, Some(1.5));
        assert_eq!(update.y, Some(64.0));
        assert_eq!(update.z, Some(-2.25));
        assert_eq!(update.yaw, None, "a position packet carries no rotation");
        assert!(update.on_ground);
    }

    #[test]
    fn a_position_and_rotation_packet_carries_both() {
        let state = MoveState {
            kind: MoveKind::PosRot,
            payload: payload(&[1.0, 2.0, 3.0], &[90.0, -45.0], false),
        };
        let update = state.decode().expect("a valid pos_rot payload");
        assert_eq!(
            (update.x, update.y, update.z),
            (Some(1.0), Some(2.0), Some(3.0))
        );
        assert_eq!(update.yaw, Some(90.0));
        assert_eq!(update.pitch, Some(-45.0));
        assert!(!update.on_ground);
    }

    #[test]
    fn a_rotation_packet_carries_only_the_angles() {
        let state = MoveState {
            kind: MoveKind::Rot,
            payload: payload(&[], &[10.0, 20.0], true),
        };
        let update = state.decode().expect("a valid rot payload");
        assert_eq!(update.x, None, "rotation does not move the player");
        assert_eq!(update.yaw, Some(10.0));
        assert_eq!(update.pitch, Some(20.0));
        assert!(update.on_ground);
    }

    #[test]
    fn a_status_packet_carries_only_the_flag() {
        let state = MoveState {
            kind: MoveKind::StatusOnly,
            payload: payload(&[], &[], false),
        };
        let update = state.decode().expect("a valid status payload");
        assert_eq!(update.x, None);
        assert_eq!(update.yaw, None);
        assert!(!update.on_ground);
    }

    #[test]
    fn a_short_payload_is_an_error_not_a_guess() {
        let state = MoveState {
            kind: MoveKind::Pos,
            payload: vec![0, 1, 2],
        };
        assert!(
            state.decode().is_err(),
            "a truncated position packet must not decode"
        );
    }
}
