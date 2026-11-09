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
//! byte is a **flags byte** (not a single boolean). The live trace
//! (`target/parity/live-session.json`) carries `0x00`, `0x01`, `0x02` and
//! `0x03` on `move_player_pos` / `move_player_pos_rot`, plus `0x01`/`0x03` on
//! `move_player_rot` and `0x01` on `move_player_status_only`.
//!
//! Bit 0 (`0x01`) is **on_ground**. The meaning of bit 1 (`0x02`) is **not
//! established** by our data, so it is not named here: the whole byte is kept
//! verbatim in [`MoveUpdate::flags`] (and recorded in the replay), never
//! dropped. The formats match what the parity walk tool writes for the live
//! client (`crates/parity/src/walk.rs`).
// SOURCE: live trace (raw byte values) + wiki (bit 0 = on ground); bit 1
// meaning unverified.

use protocol::{CodecError, PacketReader};

use super::messages::MoveState;

/// The values one movement packet carries; an absent axis is not touched.
///
/// [`MoveKind::Pos`](super::MoveKind::Pos) sets the position, `Rot` the
/// angles, `PosRot` both and `StatusOnly` only the flags byte, so the tick can
/// overlay them on the player's current state.
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
    /// The trailing flags byte, kept verbatim. Bit 0 is on-ground
    /// ([`Self::ON_GROUND`]); bit 1 (`0x02`) has no established meaning in our
    /// data and is preserved as-is.
    pub flags: u8,
}

impl MoveUpdate {
    /// Bit 0 of [`flags`](Self::flags): the client claims to stand on the
    /// ground.
    pub const ON_GROUND: u8 = 0x01;

    /// Whether bit 0 of the flags byte is set.
    pub fn on_ground(&self) -> bool {
        self.flags & Self::ON_GROUND != 0
    }
}

impl MoveState {
    /// Parses the payload according to the packet kind.
    ///
    /// An incomplete or over-long payload is an error rather than a guess:
    /// the tick logs it and drops the movement (the connection never
    /// validated the length). The flags byte is never rejected: every value
    /// the client sends (0x00..=0x03 observed) is valid.
    pub fn decode(&self) -> Result<MoveUpdate, CodecError> {
        let mut reader = PacketReader::new(&self.payload);
        let update = match self.kind {
            super::messages::MoveKind::Pos => MoveUpdate {
                x: Some(reader.read_f64()?),
                y: Some(reader.read_f64()?),
                z: Some(reader.read_f64()?),
                yaw: None,
                pitch: None,
                flags: reader.read_u8()?,
            },
            super::messages::MoveKind::PosRot => MoveUpdate {
                x: Some(reader.read_f64()?),
                y: Some(reader.read_f64()?),
                z: Some(reader.read_f64()?),
                yaw: Some(reader.read_f32()?),
                pitch: Some(reader.read_f32()?),
                flags: reader.read_u8()?,
            },
            super::messages::MoveKind::Rot => MoveUpdate {
                x: None,
                y: None,
                z: None,
                yaw: Some(reader.read_f32()?),
                pitch: Some(reader.read_f32()?),
                flags: reader.read_u8()?,
            },
            super::messages::MoveKind::StatusOnly => MoveUpdate {
                x: None,
                y: None,
                z: None,
                yaw: None,
                pitch: None,
                flags: reader.read_u8()?,
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

    /// Encodes a payload of big-endian doubles, floats and the flags byte.
    fn payload(f64s: &[f64], f32s: &[f32], flags: u8) -> Vec<u8> {
        let mut out = Vec::new();
        for value in f64s {
            out.extend_from_slice(&value.to_be_bytes());
        }
        for value in f32s {
            out.extend_from_slice(&value.to_be_bytes());
        }
        out.push(flags);
        out
    }

    #[test]
    fn a_position_packet_carries_the_position_and_the_flags() {
        let state = MoveState {
            kind: MoveKind::Pos,
            payload: payload(&[1.5, 64.0, -2.25], &[], 0x01),
        };
        let update = state.decode().expect("a valid pos payload");
        assert_eq!(update.x, Some(1.5));
        assert_eq!(update.y, Some(64.0));
        assert_eq!(update.z, Some(-2.25));
        assert_eq!(update.yaw, None, "a position packet carries no rotation");
        assert_eq!(update.flags, 0x01);
        assert!(update.on_ground());
    }

    #[test]
    fn a_position_and_rotation_packet_carries_both() {
        let state = MoveState {
            kind: MoveKind::PosRot,
            payload: payload(&[1.0, 2.0, 3.0], &[90.0, -45.0], 0x00),
        };
        let update = state.decode().expect("a valid pos_rot payload");
        assert_eq!(
            (update.x, update.y, update.z),
            (Some(1.0), Some(2.0), Some(3.0))
        );
        assert_eq!(update.yaw, Some(90.0));
        assert_eq!(update.pitch, Some(-45.0));
        assert_eq!(update.flags, 0x00);
        assert!(!update.on_ground());
    }

    #[test]
    fn a_rotation_packet_carries_only_the_angles() {
        let state = MoveState {
            kind: MoveKind::Rot,
            payload: payload(&[], &[10.0, 20.0], 0x01),
        };
        let update = state.decode().expect("a valid rot payload");
        assert_eq!(update.x, None, "rotation does not move the player");
        assert_eq!(update.yaw, Some(10.0));
        assert_eq!(update.pitch, Some(20.0));
        assert!(update.on_ground());
    }

    #[test]
    fn a_status_packet_carries_only_the_flags() {
        let state = MoveState {
            kind: MoveKind::StatusOnly,
            payload: payload(&[], &[], 0x00),
        };
        let update = state.decode().expect("a valid status payload");
        assert_eq!(update.x, None);
        assert_eq!(update.yaw, None);
        assert_eq!(update.flags, 0x00);
        assert!(!update.on_ground());
    }

    /// The live client sends 0x00..=0x03 on `move_player_pos` and
    /// `move_player_pos_rot`; none of them may be rejected, every one keeps
    /// the byte verbatim, and bit 0 alone decides on-ground.
    #[test]
    fn every_flag_of_pos_and_pos_rot_decodes_with_bit_zero_as_on_ground() {
        for flags in 0x00u8..=0x03 {
            let pos = MoveState {
                kind: MoveKind::Pos,
                payload: payload(&[1.0, 2.0, 3.0], &[], flags),
            };
            let update = pos.decode().expect("0x00..=0x03 on pos must decode");
            assert_eq!(update.flags, flags, "the flags byte is kept verbatim");
            assert_eq!(
                update.on_ground(),
                (flags & MoveUpdate::ON_GROUND) != 0,
                "bit 0 is on-ground"
            );
            assert_eq!(
                (update.x, update.y, update.z),
                (Some(1.0), Some(2.0), Some(3.0))
            );

            let pos_rot = MoveState {
                kind: MoveKind::PosRot,
                payload: payload(&[1.0, 2.0, 3.0], &[10.0, -20.0], flags),
            };
            let update = pos_rot
                .decode()
                .expect("0x00..=0x03 on pos_rot must decode");
            assert_eq!(update.flags, flags, "the flags byte is kept verbatim");
            assert_eq!(
                update.on_ground(),
                (flags & MoveUpdate::ON_GROUND) != 0,
                "bit 0 is on-ground"
            );
            assert_eq!(update.yaw, Some(10.0));
            assert_eq!(update.pitch, Some(-20.0));
        }
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
