//! The player fields the server models, in both directions.
//!
//! The vanilla playerdata document carries far more (inventory, effects,
//! attributes, the recipe book, ...). The server reads only the fields it
//! needs to place and announce a returning player and rewrites only those,
//! leaving every unknown tag of an existing file untouched.

use protocol::Uuid;

/// Position and orientation of a player (the fields of `Pos` and `Rotation`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// X coordinate in blocks.
    pub x: f64,
    /// Y coordinate (feet) in blocks.
    pub y: f64,
    /// Z coordinate in blocks.
    pub z: f64,
    /// Yaw in degrees.
    pub yaw: f32,
    /// Pitch in degrees.
    pub pitch: f32,
}

/// The part of an existing player file the join sequence reads.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedPlayer {
    /// Saved position and rotation.
    pub pose: Pose,
    /// Health; a file with `<= 0` or a non-number is read as 20 with a
    /// warning (death and respawn are not modelled).
    pub health: f32,
    /// Food level, 0..=20.
    pub food: i32,
    /// Food saturation.
    pub saturation: f32,
    /// `OnGround` byte.
    pub on_ground: bool,
    /// `playerGameType`, if it is one of the four vanilla modes.
    ///
    /// `None` means the file has no usable mode, so the configured mode is
    /// used even when `force_gamemode` is off.
    pub game_mode: Option<i32>,
    /// `Dimension` as stored; only `minecraft:overworld` is joinable.
    pub dimension: String,
}

impl SavedPlayer {
    /// Whether the file belongs to this server's only dimension.
    pub fn is_overworld(&self) -> bool {
        self.dimension == OVERWORLD
    }
}

/// The dimension this server serves.
pub const OVERWORLD: &str = "minecraft:overworld";

/// The state one save operation writes: what the tick thread snapshots.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Snapshot {
    /// Offline UUID of the player (also selects the file name).
    pub uuid: Uuid,
    /// Current position and rotation.
    pub pose: Pose,
    /// Whether the player stood on the ground at the snapshot.
    pub on_ground: bool,
    /// Current health, 0..=20.
    pub health: f32,
    /// Current food level, 0..=20.
    pub food: i32,
    /// Current saturation.
    pub saturation: f32,
    /// Current game mode (0..=3).
    pub game_mode: i32,
}

/// The values a brand-new player starts with (the vanilla S7 trace of a
/// player without a file: `docs/PLAYERDATA.md` §1.3).
pub const NEW_PLAYER_HEALTH: f32 = 20.0;
/// Food level of a brand-new player.
pub const NEW_PLAYER_FOOD: i32 = 20;
/// Saturation of a brand-new player.
pub const NEW_PLAYER_SATURATION: f32 = 5.0;
