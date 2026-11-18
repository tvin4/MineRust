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

/// One stack of the saved `Inventory` list (Шаг 24d).
///
/// Vanilla stores the list as compounds `{count: int, Slot: byte, id: string}`
/// (Шаг 24.0d read a real 26.3 file); components are not modelled and a stack
/// that carries any is not written back (see `store::update`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedItem {
    /// `Slot` byte; the player-file slot space (`0..=8` hotbar, `9..=35`
    /// main, `36..=39` armor, `40` offhand).
    pub slot: i32,
    /// `count` int; `1..=99` is what the server writes.
    pub count: i32,
    /// `id` string, e.g. `minecraft:stone`.
    pub id: String,
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
    /// `Inventory` entries the server reads back (Шаг 24d). `Slot` is the
    /// player-file slot space; `update` writes the hotbar back into it.
    pub inventory: Vec<SavedItem>,
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
#[derive(Debug, Clone, PartialEq)]
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
    /// Hotbar stacks to write into `Inventory` (Шаг 24d). The player-file
    /// slot space is `0..=8`; entries of other slots in an existing file are
    /// kept as they are.
    pub inventory: Vec<SavedItem>,
}

/// The values a brand-new player starts with (the vanilla S7 trace of a
/// player without a file: `docs/PLAYERDATA.md` §1.3).
pub const NEW_PLAYER_HEALTH: f32 = 20.0;
/// Food level of a brand-new player.
pub const NEW_PLAYER_FOOD: i32 = 20;
/// Saturation of a brand-new player.
pub const NEW_PLAYER_SATURATION: f32 = 5.0;
