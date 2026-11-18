//! One region owns its players (ARCH-M4 §1).
//!
//! M4.1 runs exactly one region; the shape (an id, a player map, a
//! `tick`) is what M4.3 turns into a pool of regions, so the scheduler
//! never changes when a second region appears.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use protocol::Uuid;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tracing::warn;

use super::entities::{Forced, StreamEntities};
use super::grid::{chunk_of_position, RegionGrid};
use super::inventory::Inventory;
use super::messages::{
    KickFlag, KickReason, MoveState, PlayerConnection, PlayerId, PlayerJoin, ServerMessage,
};
use super::movement::MoveUpdate;
use crate::chunks::window_difference;

/// Id of a region. M4.1 has exactly one, with the id in [`SINGLE_REGION`].
pub type RegionId = i32;

/// Ticks one swing lasts (M4.5 Шаг 11).
///
/// A vanilla-memory hypothesis the S3 trace is consistent with (the tick
/// model reproduces its cluster structure); the trace itself does not pin
/// the duration.
pub const SWING_DURATION_TICKS: i32 = 6;

/// Ticks the swing timer must run before the next punch is accepted
/// (M4.5 Шаг 11): the punch rule accepts when `swing_time >= ` this value.
///
/// **Не определено трейсом**: the S3 trace fits a threshold of 3 and of 4
/// ticks equally well (the model finds matches for both). 3 — half the
/// 6-tick duration — is the implemented choice.
pub const SWING_THRESHOLD_TICKS: i32 = 3;

/// The only region of M4.1. M4.3 replaces it with a chunk → region map.
pub const SINGLE_REGION: RegionId = 0;

/// Precise position and orientation of a player, as the movement packets
/// carry it (double coordinates, float angles, ARCH-M4 §0/§2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerPos {
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
    /// Head yaw in degrees (M4.5 Шаг 8). The serverbound movement packets
    /// carry only the body yaw/pitch, so no separate head source is
    /// established: the state tracks it as following the body yaw. The
    /// encoders that need it (`rotate_head`) read this field.
    pub head_yaw: f32,
    /// The raw movement flags byte the client sent (bit 0: on the ground,
    /// [`PlayerPos::on_ground`]; other bits are preserved, not interpreted).
    pub flags: u8,
}

impl PlayerPos {
    /// The entry position: the centre of the spawn block, the same point the
    /// join teleport sends (`join.rs`).
    ///
    /// Used when a player has no saved pose (`PlayerJoin::pose`, Шаг 18a) and
    /// by the join's fallback for a save the server refuses (a non-overworld
    /// dimension).
    pub(crate) fn entry(spawn: protocol::Position) -> Self {
        Self {
            x: f64::from(spawn.x) + 0.5,
            y: f64::from(spawn.y),
            z: f64::from(spawn.z) + 0.5,
            yaw: 0.0,
            pitch: 0.0,
            head_yaw: 0.0,
            flags: 0,
        }
    }

    /// Bit 0 of [`flags`](Self::flags): the player stands on the ground.
    pub fn on_ground(&self) -> bool {
        self.flags & MoveUpdate::ON_GROUND != 0
    }

    /// Overlays the axes a movement packet carries onto this position.
    fn apply(&mut self, update: &MoveUpdate) {
        if let Some(x) = update.x {
            self.x = x;
        }
        if let Some(y) = update.y {
            self.y = y;
        }
        if let Some(z) = update.z {
            self.z = z;
        }
        if let Some(yaw) = update.yaw {
            self.yaw = yaw;
            // No separate head source is established (see `head_yaw`): the
            // head follows the body yaw.
            self.head_yaw = yaw;
        }
        if let Some(pitch) = update.pitch {
            self.pitch = pitch;
        }
        self.flags = update.flags;
    }

    /// Overlays only the angles of a rotation-carrying packet (Шаг 19a).
    ///
    /// The rotation slot may hold an older packet than the movement slot (a
    /// `move_player_rot` followed by `move_player_pos` in the same tick), so
    /// its position and flags must not overwrite the newer movement; the
    /// angles are exactly what the newer packet does not carry.
    fn apply_rotation(&mut self, update: &MoveUpdate) {
        if let Some(yaw) = update.yaw {
            self.yaw = yaw;
            self.head_yaw = yaw;
        }
        if let Some(pitch) = update.pitch {
            self.pitch = pitch;
        }
    }
}

/// A survival dig the server is timing (Шаг 16a.3).
///
/// One per player: vanilla has a single active dig at a time. The state is the
/// server's own progress estimate, not a mirror of the client's prediction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DigState {
    /// Block being dug.
    pub position: protocol::Position,
    /// Progress of the dig; `1.0` is the expected finish
    /// (`30·hardness/speed` ticks, §1.7 of `docs/BLOCK-BREAKING.md`).
    pub progress: f32,
    /// Progress added per tick (`speed/hardness/30`), kept in `f32` like the
    /// game's own float accumulation (a `f64` sum of six `1/6` steps lands a
    /// hair below `1.0` and misses the traced stage `10`).
    pub step: f32,
    /// Stage byte the observers already saw; `None` before the first stage.
    pub stage: Option<u8>,
    /// Ticks the dig has run (diagnostics and the finish-time tolerance).
    pub ticks: u32,
}

/// A player the game thread owns: routing, whether movement may be trusted
/// and the newest applied position (M4.2).
#[derive(Debug)]
pub struct Player {
    id: PlayerId,
    name: String,
    /// Offline UUID of the player's name (Шаг 18a): the save file is named
    /// after it, so it is derived once at join instead of per save.
    uuid: Uuid,
    view_distance: i32,
    /// Game mode of this player (M4.5 Шаг 7.3): the login `game_mode`, the
    /// tab-list entry and the mode-dependent metadata bytes all come from it.
    game_mode: i32,
    /// Operator level of the session (Шаг 25b): 0 or [`super::OP_LEVEL`].
    /// Commands above the level are unknown to the client, like vanilla hides
    /// them from the tree.
    op_level: i32,
    teleport_confirmed: bool,
    spawn: protocol::Position,
    position: PlayerPos,
    /// Body yaw at the end of the previous server tick.
    ///
    /// The wire head yaw is this value, not the current one: in the vanilla C4
    /// trace every `move_entity_rot`/`rotate_head` pair carries the body yaw of
    /// the previous tick (head lags the body by one input step; 44/50 pairs
    /// match a previous input exactly, the rest are sync/spawn packets). The
    /// visibility pass snapshots this into [`PlayerPos::head_yaw`] and advances
    /// it once per tick.
    // SOURCE: two-bot vanilla trace 26.3 (C4), byte-level pair analysis.
    previous_yaw: f32,
    /// Health the join carried (Шаг 18a). Death and respawn are not modelled,
    /// so it stays what the join announced; every save writes it back.
    health: f32,
    /// Food level the join carried (Шаг 18a).
    food: i32,
    /// Food saturation the join carried (Шаг 18a).
    saturation: f32,
    /// Chunk the client's chunk-cache centre is at (M4.4 B): the join spawn
    /// chunk first, then the chunk of the newest applied move. The tick
    /// derives a [`StreamChunks`] event whenever this changes.
    window_center: (i32, i32),
    /// Other players this one currently sees, with their offline UUIDs
    /// (M4.5 Шаг 3). Part of the player, so a transfer carries it; the tick's
    /// visibility pass keeps it in step with the world.
    visible: BTreeMap<PlayerId, Uuid>,
    /// The pose last broadcast to every current viewer of this player, or
    /// `None` before the first spawn. The next movement delta is measured
    /// from it.
    last_broadcast: Option<PlayerPos>,
    /// Server ticks since this player's pose was last broadcast, reset by a
    /// spawn or any movement packet (M4.5 Шаг 6.2). The visibility pass
    /// increments it every tick, sends a pose change only every
    /// [`UPDATE_INTERVAL`](super::entities::UPDATE_INTERVAL) ticks and a
    /// periodic zero-delta `move_entity_pos` when it reaches
    /// [`BROADCAST_PERIOD_TICKS`](super::entities::BROADCAST_PERIOD_TICKS) — the
    /// vanilla cadence of the two-bot trace (C1/C2/C4).
    ticks_since_broadcast: u32,
    /// Whether the player is sneaking (M4.5 step 8). The source of truth is
    /// the `player_input` flags byte ([`MoveSlot::take_input`]); the
    /// `player_command` packet carries only sprint actions in the trace.
    sneak: bool,
    /// Whether the player is sprinting (M4.5 step 8, corrected by Шаг 15).
    /// The source of truth is the `player_command` start/stop action
    /// ([`MoveSlot::take_command`]); the `player_input` `0x40` bit is only the
    /// held key and is not used for the flag (the r3 trace keeps the flag after
    /// the key release, until the stop action arrives).
    sprint: bool,
    /// Whether the player's feet are in water (Шаг 14d, `docs/SWIMMING.md`).
    /// Recomputed every tick from the shared `ChunkStore` by the scheduler's
    /// update pass — a region owns only players, the store is the
    /// scheduler's field.
    swimming: bool,
    /// Pose forced by the free headroom above the feet (Шаг 15 п.5),
    /// recomputed every tick from the shared `ChunkStore`. Unlike `sneaking`,
    /// this is not an input: vanilla lowers the pose when the player no longer
    /// fits. [`Forced::None`] means the store could not decide (no collision
    /// report) or the player fits standing.
    forced: Forced,
    /// The sneak/sprint/swim/forced state currently represented in each
    /// viewer's metadata. `None` means no observer state has been sent yet; an
    /// initial sneaking spawn carries the trace-confirmed flags and pose (S7),
    /// while sprint spawn encoding is not established.
    broadcast_state: Option<(bool, bool, bool, Forced)>,
    /// Swing animation state (M4.5 Шаг 11): whether a swing is in progress
    /// and how many ticks it has run. The rule and the constants are a
    /// vanilla-memory hypothesis confirmed by the tick model against the S3
    /// trace (`tools/trace-analyze/m45-step11-swing-model.py`).
    swinging: bool,
    /// Ticks the current swing has run; `-1` marks the tick it started in,
    /// so a punch in the same tick still counts as "just started".
    swing_time: i32,
    /// Swings accepted this tick, awaiting the visibility pass broadcast:
    /// one `swing_animation` packet per entry, to every observer (M4.5
    /// Шаг 11). The packet itself goes only to observers, never to the
    /// player (the S3 trace shows A receiving none).
    pending_swings: u32,
    /// The survival dig this player is timing (Шаг 16a.3), if any. Creative
    /// and instant (`hardness = 0`) breaks never set it.
    dig: Option<DigState>,
    /// The 46-slot player inventory and the selected hotbar slot (Шаг 24a).
    inventory: Inventory,
    connection: PlayerConnection,
}

impl Player {
    /// Builds a player from a join registration.
    ///
    /// The entry pose is the saved one carried by the join when there is one
    /// (Шаг 18a), otherwise the centre of the spawn block. The head yaw
    /// follows the body yaw (no separate head source, M4.5 Шаг 8).
    pub fn new(join: PlayerJoin) -> Self {
        let mut position = join.pose.unwrap_or_else(|| PlayerPos::entry(join.spawn));
        position.head_yaw = position.yaw;
        let window_center = chunk_of_position(position.x, position.z);
        Self {
            // Read before `join.name` is moved into the field below.
            uuid: protocol::offline_uuid(&join.name),
            id: join.id,
            name: join.name,
            view_distance: join.view_distance,
            game_mode: join.game_mode,
            op_level: join.op_level,
            teleport_confirmed: false,
            spawn: join.spawn,
            previous_yaw: position.yaw,
            position,
            health: join.health,
            food: join.food,
            saturation: join.saturation,
            window_center,
            visible: BTreeMap::new(),
            last_broadcast: None,
            ticks_since_broadcast: 0,
            sneak: false,
            sprint: false,
            swimming: false,
            forced: Forced::None,
            broadcast_state: None,
            swinging: false,
            swing_time: 0,
            pending_swings: 0,
            dig: None,
            inventory: Inventory::empty(),
            connection: join.connection,
        }
    }

    /// Player id.
    pub fn id(&self) -> PlayerId {
        self.id
    }

    /// Offline UUID of the player (Шаг 18a).
    pub fn uuid(&self) -> Uuid {
        self.uuid
    }

    /// Health the join carried (Шаг 18a); written back on every save.
    pub fn health(&self) -> f32 {
        self.health
    }

    /// Food level the join carried (Шаг 18a).
    pub fn food(&self) -> i32 {
        self.food
    }

    /// Food saturation the join carried (Шаг 18a).
    pub fn saturation(&self) -> f32 {
        self.saturation
    }

    /// Game mode of this player (M4.5 Шаг 7.3).
    pub fn game_mode(&self) -> i32 {
        self.game_mode
    }

    /// Changes the game mode of this player (Шаг 25b/25c), returning whether
    /// it actually changed. The tab list, the ability packets and the
    /// spectator visibility rule are the caller's business; this keeps the
    /// state the tick reads.
    pub fn set_game_mode(&mut self, game_mode: i32) -> bool {
        let changed = self.game_mode != game_mode;
        self.game_mode = game_mode;
        changed
    }

    /// Operator level of the session (Шаг 25b).
    pub fn op_level(&self) -> i32 {
        self.op_level
    }

    /// Teleports the player to an absolute position (Шаг 25b `/tp`): the
    /// pose and the chunk-cache centre move, and the next visibility pass is
    /// forced to broadcast the new position (a large jump becomes
    /// `entity_position_sync`).
    pub fn teleport(&mut self, x: f64, y: f64, z: f64) {
        self.position.x = x;
        self.position.y = y;
        self.position.z = z;
        self.window_center = chunk_of_position(x, z);
        self.ticks_since_broadcast = super::entities::UPDATE_INTERVAL;
    }

    /// The player's inventory (Шаг 24a).
    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }

    /// The player's inventory, mutable (creative writes, pick, placement).
    pub fn inventory_mut(&mut self) -> &mut Inventory {
        &mut self.inventory
    }

    /// Replaces the inventory, used once at join to restore the saved hotbar
    /// (Шаг 24d).
    pub fn set_inventory(&mut self, inventory: Inventory) {
        self.inventory = inventory;
    }

    /// The survival dig this player is timing (Шаг 16a.3), if any.
    pub fn dig(&self) -> Option<DigState> {
        self.dig
    }

    /// Mutable access to the dig state for the per-tick advance.
    pub fn dig_mut(&mut self) -> Option<&mut DigState> {
        self.dig.as_mut()
    }

    /// Replaces the dig state (a start adopts the new block).
    pub fn set_dig(&mut self, dig: Option<DigState>) {
        self.dig = dig;
    }

    /// Stops timing the dig and returns the state that was tracked.
    pub fn take_dig(&mut self) -> Option<DigState> {
        self.dig.take()
    }

    /// Name of Login Start.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// View distance the client asked for, in chunks.
    pub fn view_distance(&self) -> i32 {
        self.view_distance
    }

    /// Block the player joined at.
    pub fn spawn(&self) -> protocol::Position {
        self.spawn
    }

    /// Whether the client confirmed the entry teleport. Movement is only
    /// meaningful after it, so M4.2 applies positions only then.
    pub fn teleport_confirmed(&self) -> bool {
        self.teleport_confirmed
    }

    /// Marks the entry teleport as confirmed.
    pub fn confirm_teleport(&mut self) {
        self.teleport_confirmed = true;
    }

    /// The newest applied position and orientation (M4.2).
    pub fn position(&self) -> PlayerPos {
        self.position
    }

    /// Body yaw at the end of the previous server tick: the head yaw the wire
    /// packets carry this tick.
    pub fn previous_yaw(&self) -> f32 {
        self.previous_yaw
    }

    /// Advances the lagged head source to this tick's body yaw (the tick's
    /// visibility pass owns it).
    pub fn set_previous_yaw(&mut self, yaw: f32) {
        self.previous_yaw = yaw;
    }

    /// The chunk the client's chunk-cache centre sits at (M4.4 B).
    pub fn window_center(&self) -> (i32, i32) {
        self.window_center
    }

    /// Other players this one currently sees, with their offline UUIDs
    /// (M4.5 Шаг 3).
    pub fn visible(&self) -> &BTreeMap<PlayerId, Uuid> {
        &self.visible
    }

    /// Replaces the visible set (the tick's visibility pass owns it).
    pub fn set_visible(&mut self, visible: BTreeMap<PlayerId, Uuid>) {
        self.visible = visible;
    }

    /// The visible set, mutably: the visibility pass adds the newly visible
    /// and drops the gone entries in place instead of rebuilding the map every
    /// tick (M4.5 Шаг 3.5).
    pub(crate) fn visible_mut(&mut self) -> &mut BTreeMap<PlayerId, Uuid> {
        &mut self.visible
    }

    /// The pose last broadcast to this player's viewers, or `None`.
    pub fn last_broadcast(&self) -> Option<PlayerPos> {
        self.last_broadcast
    }

    /// Replaces the last broadcast pose (the tick's visibility pass owns it).
    pub fn set_last_broadcast(&mut self, pose: Option<PlayerPos>) {
        self.last_broadcast = pose;
    }

    /// Server ticks since this player's pose was last broadcast (M4.5 Шаг
    /// 6.2); the visibility pass owns it.
    pub fn ticks_since_broadcast(&self) -> u32 {
        self.ticks_since_broadcast
    }

    /// Replaces the broadcast tick counter (the tick's visibility pass owns
    /// it).
    pub fn set_ticks_since_broadcast(&mut self, ticks: u32) {
        self.ticks_since_broadcast = ticks;
    }

    /// Applies a decoded movement packet to this player.
    ///
    /// No movement is validated in M4 (ARCH-M4 §0): the values are accepted
    /// as they arrive.
    pub fn apply_move(&mut self, update: &MoveUpdate) {
        self.position.apply(update);
    }

    /// Applies only the angles of a decoded rotation-carrying packet (Шаг
    /// 19a); see [`PlayerPos::apply_rotation`].
    pub fn apply_rotation(&mut self, update: &MoveUpdate) {
        self.position.apply_rotation(update);
    }

    /// Whether the player is sneaking (M4.5 step 8).
    pub fn sneak(&self) -> bool {
        self.sneak
    }

    /// Whether the player is sprinting (M4.5 step 8).
    pub fn sprint(&self) -> bool {
        self.sprint
    }

    /// Whether the player's feet are in water (Шаг 14d).
    pub fn swimming(&self) -> bool {
        self.swimming
    }

    /// Sets the water state from the scheduler's store pass (Шаг 14d); the
    /// visibility pass turns a change into flags/pose metadata.
    pub(crate) fn set_swimming(&mut self, swimming: bool) {
        self.swimming = swimming;
    }

    /// The pose forced by the free headroom above the feet (Шаг 15 п.5).
    pub fn forced(&self) -> Forced {
        self.forced
    }

    /// Sets the forced pose from the scheduler's store pass; the visibility
    /// pass turns a change into flags/pose metadata.
    pub(crate) fn set_forced(&mut self, forced: Forced) {
        self.forced = forced;
    }

    /// State flags last sent to observers, if initialized by a spawn.
    pub(crate) fn broadcast_state(&self) -> Option<(bool, bool, bool, Forced)> {
        self.broadcast_state
    }

    /// Updates the observer-state baseline after the visibility pass.
    pub(crate) fn set_broadcast_state(&mut self, state: Option<(bool, bool, bool, Forced)>) {
        self.broadcast_state = state;
    }

    /// Swings accepted this tick, not yet broadcast (M4.5 Шаг 11); the
    /// visibility pass owns it.
    pub fn pending_swings(&self) -> u32 {
        self.pending_swings
    }

    /// Replaces the pending swing broadcast count (the visibility pass owns
    /// it).
    pub(crate) fn set_pending_swings(&mut self, count: u32) {
        self.pending_swings = count;
    }

    /// Applies `count` new `punch` packets (M4.5 Шаг 11) and returns how
    /// many produced a swing this tick.
    ///
    /// Rule (a vanilla-memory hypothesis, confirmed by the tick model
    /// against the S3 trace, `tools/trace-analyze/m45-step11-swing-model.py`):
    /// a punch is accepted when the player is not swinging, or the swing
    /// just started (`swing_time < 0`), or the previous swing is old enough
    /// ([`SWING_THRESHOLD_TICKS`]). An accepted punch starts a new swing;
    /// punches during a young swing are dropped — that is why 22 punches
    /// produced only 11 swings in the trace.
    pub fn apply_punches(&mut self, count: u32) -> u32 {
        let mut accepted = 0;
        for _ in 0..count {
            if !self.swinging || self.swing_time >= SWING_THRESHOLD_TICKS || self.swing_time < 0 {
                self.swinging = true;
                self.swing_time = -1;
                accepted += 1;
            }
        }
        self.pending_swings = self.pending_swings.saturating_add(accepted);
        accepted
    }

    /// Advances the swing timer at the end of a tick (M4.5 Шаг 11).
    ///
    /// The packets of the tick are processed before this runs. At
    /// [`SWING_DURATION_TICKS`] the swing ends and the timer resets; the
    /// duration is a vanilla-memory hypothesis the trace is consistent with.
    pub fn advance_swing(&mut self) {
        if !self.swinging {
            return;
        }
        self.swing_time += 1;
        if self.swing_time >= SWING_DURATION_TICKS {
            self.swinging = false;
            self.swing_time = 0;
        }
    }

    /// Whether a swing is in progress (M4.5 Шаг 11).
    pub fn swinging(&self) -> bool {
        self.swinging
    }

    /// Ticks the current swing has run (`-1` in its first tick).
    pub fn swing_time(&self) -> i32 {
        self.swing_time
    }
    /// Applies the newest `player_input` flags byte (M4.5 step 8).
    ///
    /// The sneak bit (`0x20`) is the crouch state. The sprint bit (`0x40`) is
    /// the held sprint key only; after Шаг 15 the sprint flag is driven by
    /// [`Self::apply_command`] instead, so the bit is deliberately not applied
    /// here. The other bits are movement keys and stay inside the byte in the
    /// connection only.
    pub fn apply_input(&mut self, flags: u8) {
        self.sneak = flags & protocol::PlayerInput::SNEAK != 0;
    }

    /// Applies a `player_command` sprint toggle (Шаг 15).
    ///
    /// The r3 trace pairs action 1 with every observed `0x08` and action 2 with
    /// every `0x00`, so the command — not the `player_input` bit — is the
    /// sprint state. Called after [`Self::apply_input`] in the tick, so a
    /// command and a bit in the same tick settle on the command.
    pub fn apply_command(&mut self, sprint: bool) {
        self.sprint = sprint;
    }

    /// The connection's outbound queue.
    pub fn outbound(&self) -> &mpsc::Sender<ServerMessage> {
        &self.connection.outbound
    }

    /// The kick flag shared with the connection.
    pub fn kick_flag(&self) -> &KickFlag {
        &self.connection.kick
    }

    /// Takes the newest movement packet, leaving the slot empty.
    pub fn take_move(&self) -> Option<MoveState> {
        self.connection.last_move.take()
    }

    /// Takes the newest packet that carried yaw/pitch, leaving the entry empty
    /// (Шаг 19a). See [`MoveSlot::take_rotation`].
    pub fn take_rotation(&self) -> Option<MoveState> {
        self.connection.last_move.take_rotation()
    }

    /// Takes the newest `player_input` flags byte, leaving the entry empty
    /// (M4.5 step 8).
    pub fn take_input(&self) -> Option<u8> {
        self.connection.last_move.take_input()
    }

    /// Takes the newest `player_command` sprint toggle, leaving the entry
    /// empty (Шаг 15).
    pub fn take_command(&self) -> Option<bool> {
        self.connection.last_move.take_command()
    }

    /// Takes and resets the swing count of the slot (M4.5 step 8).
    pub fn take_swings(&self) -> u32 {
        self.connection.last_move.take_swings()
    }
}

/// A player the region removed, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kick {
    /// Region that removed the player.
    pub region: RegionId,
    /// Player that was removed.
    pub id: PlayerId,
    /// Reason it was removed.
    pub reason: KickReason,
}

/// A position a region applied from a movement packet this tick (M4.2).
///
/// The tick returns these instead of writing the replay itself, so the
/// region stays generic over the journal sink and the game loop owns the
/// recording order (`(tick, seq)` is per event, ARCH-M4 §1). `seq` is the
/// region's own event counter this tick (moves and transfers share it, so the
/// order inside one region is the order the region produced them).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AppliedMove {
    /// Region that applied the move.
    pub region: RegionId,
    /// Player whose position moved.
    pub id: PlayerId,
    /// Order of the event inside the region this tick.
    pub seq: u32,
    /// The player's position after the move.
    pub position: PlayerPos,
}

/// A player that crossed a region boundary this tick (M4.3 part B).
///
/// The move was applied **in the source region** ([`AppliedMove`] carries it);
/// the player itself stays there until the scheduler applies the transfer
/// after the barrier. `seq` is the source region's event counter, so the
/// canonical order across regions is `(from, seq)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transfer {
    /// Player that crossed the boundary.
    pub player: PlayerId,
    /// Region that owned the player this tick.
    pub from: RegionId,
    /// Region that owns the player from the next tick.
    pub to: RegionId,
    /// Order of the event inside the source region this tick.
    pub seq: u32,
}

/// A player whose chunk window moved this tick (M4.4 B, ШАГ 2).
///
/// The region emits it the moment an applied move changes the player's cache
/// centre chunk; the connection turns it into `set_chunk_cache_center`,
/// `forget_level_chunk` and the next chunk batch (ШАГ 3). `seq` is the
/// region's event counter, shared with moves and transfers. The connection
/// owns the "what was really sent" bookkeeping; the region only says what
/// the window wants, so the tick stays pure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamChunks {
    /// Region that owns the player this tick.
    pub region: RegionId,
    /// Player whose window moved.
    pub player: PlayerId,
    /// Order of the event inside the region this tick.
    pub seq: u32,
    /// New cache centre, in chunks.
    pub center: (i32, i32),
    /// Chunks entering the window, in the new centre's send order.
    pub add: Vec<(i32, i32)>,
    /// Chunks leaving the window, ascending `(x, z)` (the traced order).
    pub remove: Vec<(i32, i32)>,
}

/// Everything the regions produced in one tick.
#[derive(Debug, Default, PartialEq)]
pub struct TickOutcome {
    /// Positions applied from movement packets, in region then player order.
    pub moves: Vec<AppliedMove>,
    /// Players kicked this tick.
    pub kicks: Vec<Kick>,
    /// Boundary crossings detected this tick, in `(from, seq)` order.
    pub transfers: Vec<Transfer>,
    /// Window moves of this tick, in `(region, seq)` order.
    pub streams: Vec<StreamChunks>,
    /// Entity-visibility changes of this tick (M4.5 Шаг 3), in `(region,
    /// seq)` order; the scheduler computes them after the transfers.
    pub entities: Vec<StreamEntities>,
}

impl TickOutcome {
    /// The canonical "outgoing" transcript of this tick (ARCH-M4 §1/§6).
    ///
    /// One compact JSON line per tick result, ordered by `(region, seq)` with
    /// moves, transfers and window moves sharing the region's event counter;
    /// entity events (M4.5 Шаг 3) and kicks follow. The transcript is the
    /// ordered list of everything the ticks produced — M4.3's positions and
    /// crossings plus M4.4 B's `stream` and M4.5's `entity` events. Used by
    /// the determinism tests and the replay check.
    pub fn transcript(&self, tick: u64) -> Vec<String> {
        let mut events: Vec<(RegionId, u32, u8, serde_json::Value)> = Vec::new();
        for moved in &self.moves {
            let value = serde_json::json!({
                "tick": tick,
                "region": moved.region,
                "kind": "move",
                "src": moved.id,
                "seq": moved.seq,
                "x": moved.position.x,
                "y": moved.position.y,
                "z": moved.position.z,
                "yaw": moved.position.yaw,
                "pitch": moved.position.pitch,
                "flags": moved.position.flags,
            });
            events.push((moved.region, moved.seq, 0, value));
        }
        for transfer in &self.transfers {
            let value = serde_json::json!({
                "tick": tick,
                "region": transfer.from,
                "kind": "transfer",
                "src": transfer.player,
                "seq": transfer.seq,
                "from": transfer.from,
                "to": transfer.to,
            });
            events.push((transfer.from, transfer.seq, 1, value));
        }
        for stream in &self.streams {
            let value = serde_json::json!({
                "tick": tick,
                "region": stream.region,
                "kind": "stream",
                "src": stream.player,
                "seq": stream.seq,
                "center": [stream.center.0, stream.center.1],
                "add": stream.add,
                "remove": stream.remove,
            });
            events.push((stream.region, stream.seq, 2, value));
        }
        for entity in &self.entities {
            let spawn: Vec<PlayerId> = entity.spawn.iter().map(|spawned| spawned.id).collect();
            let remove: Vec<PlayerId> = entity.remove.iter().map(|removed| removed.id).collect();
            let moves: Vec<serde_json::Value> = entity
                .moves
                .iter()
                .map(|moved| {
                    serde_json::json!({
                        "id": moved.id,
                        "motion": moved.motion.name(),
                    })
                })
                .collect();
            let value = serde_json::json!({
                "tick": tick,
                "region": entity.region,
                "kind": "entity",
                "src": entity.player,
                "seq": entity.seq,
                "spawn": spawn,
                "remove": remove,
                "move": moves,
            });
            events.push((entity.region, entity.seq, 3, value));
        }
        for kick in &self.kicks {
            let value = serde_json::json!({
                "tick": tick,
                "region": kick.region,
                "kind": "kick",
                "src": kick.id,
                "reason": kick.reason.message(),
            });
            events.push((kick.region, u32::MAX, 4, value));
        }
        events.sort_by_key(|(region, seq, kind, _)| (*region, *seq, *kind));
        events
            .into_iter()
            .map(|(_, _, _, value)| value.to_string())
            .collect()
    }
}

/// One region: its id and the players it owns.
///
/// Players live in a [`BTreeMap`] keyed by id, so every traversal (tick,
/// transfer detection, outgoing) is in ascending player id — a `HashMap`
/// would iterate in a per-process random order and break the determinism the
/// replay test checks (ARCH-M4 §1, M4.3 part B).
#[derive(Debug)]
pub struct Region {
    id: RegionId,
    players: BTreeMap<PlayerId, Player>,
    /// Test-only slowdown of [`Region::tick`] (ordering measurement of the
    /// pool); zero in production.
    #[cfg(test)]
    delay: Duration,
    /// Synthetic per-region workload of the M4.6 benchmark (ARCH-M4 §5,
    /// W2): [`Region::tick`] busy-loops this long before doing its real
    /// work. Zero everywhere but the bench; **not** game speed — it is the
    /// scalability ceiling of the pool mechanism, not of the game.
    workload: Duration,
}

impl Region {
    /// A region with no players.
    pub fn new(id: RegionId) -> Self {
        Self {
            id,
            players: BTreeMap::new(),
            #[cfg(test)]
            delay: Duration::ZERO,
            workload: Duration::ZERO,
        }
    }

    /// Sets the synthetic W2 workload of the M4.6 benchmark: the tick
    /// busy-loops this long per region before its real work (ARCH-M4 §5).
    ///
    /// The loop is `std::hint::black_box` arithmetic, not a sleep: it burns
    /// the same kind of CPU time the real region work would, so the pooled
    /// path is measured under load. **Not** game speed — see the field doc.
    pub fn set_bench_workload(&mut self, workload: Duration) {
        self.workload = workload;
    }

    /// Region id.
    pub fn id(&self) -> RegionId {
        self.id
    }

    /// Number of players the region owns.
    pub fn player_count(&self) -> usize {
        self.players.len()
    }

    /// Whether the region owns no players.
    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }

    /// Whether a player is in this region.
    pub fn contains(&self, id: PlayerId) -> bool {
        self.players.contains_key(&id)
    }

    /// The ids the region owns, in ascending order.
    pub fn player_ids(&self) -> impl Iterator<Item = PlayerId> + '_ {
        self.players.keys().copied()
    }

    /// The players the region owns, in ascending id (autosave, diagnostics).
    pub fn players(&self) -> impl Iterator<Item = &Player> + '_ {
        self.players.values()
    }

    /// Adds a player; returns the one already under that id, if any.
    pub fn insert(&mut self, join: PlayerJoin) -> Option<Player> {
        self.players.insert(join.id, Player::new(join))
    }

    /// Puts an already-built player in (a transfer moves the whole player by
    /// ownership); returns the one already under that id, if any.
    pub fn put(&mut self, player: Player) -> Option<Player> {
        self.players.insert(player.id(), player)
    }

    /// Removes a player.
    pub fn remove(&mut self, id: PlayerId) -> Option<Player> {
        self.players.remove(&id)
    }

    /// Looks a player up.
    pub fn get(&self, id: PlayerId) -> Option<&Player> {
        self.players.get(&id)
    }

    /// Looks a player up, mutably.
    pub fn get_mut(&mut self, id: PlayerId) -> Option<&mut Player> {
        self.players.get_mut(&id)
    }

    /// Ticks the region once: applies the newest movement of every player and
    /// detects the boundary crossings those moves caused.
    ///
    /// A movement packet is applied only after the client confirmed the
    /// entry teleport ([`Player::teleport_confirmed`]); until then it is
    /// dropped (ARCH-M4 §0). No movement is validated in M4. Kicks are not
    /// produced here — they come from [`Region::deliver`], which M4.4's
    /// streaming calls.
    ///
    /// Players are visited in ascending id ([`BTreeMap`] order), so the
    /// per-region event counter `seq` — shared by moves and transfers — is
    /// deterministic. A move that lands the player in another region is still
    /// applied **here** (the position is updated and returned as an
    /// [`AppliedMove`]); the region only reports the [`Transfer`], it never
    /// hands the player over mid-tick. The scheduler applies the transfers
    /// after the barrier.
    pub fn tick(&mut self, _tick: u64, _now: Instant, grid: RegionGrid) -> TickOutcome {
        #[cfg(test)]
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        // The W2 workload is "S μs per region that owns at least one player"
        // (BENCH-M4 §2): an empty region that survived a transfer does no
        // synthetic work, exactly like a region with nothing to do.
        if !self.workload.is_zero() && !self.players.is_empty() {
            spin(self.workload);
        }
        let mut outcome = TickOutcome::default();
        let mut seq: u32 = 0;
        // `BTreeMap` iterates keyed by id; the borrow ends before the transfer
        // is pushed, so the region stays the sole owner of the players.
        for (&id, player) in self.players.iter_mut() {
            // Input flags and punches are independent of movement (M4.5
            // step 8): a tick with no movement packet still applies them.
            // The connection's slot is the queue; the player's fields are
            // the applied state. The punch packets of the tick are
            // processed BEFORE the swing timer advances (M4.5 Шаг 11); the
            // accepted swings queue for the visibility pass broadcast.
            if let Some(flags) = player.take_input() {
                player.apply_input(flags);
            }
            // The `player_command` sprint toggle is applied after the input
            // byte (Шаг 15): the command, not the `0x40` key bit, is the flag.
            if let Some(sprint) = player.take_command() {
                player.apply_command(sprint);
            }
            let punches = player.take_swings();
            if punches > 0 {
                player.apply_punches(punches);
            }
            player.advance_swing();
            let rotation = player.take_rotation();
            let movement = player.take_move();
            if rotation.is_none() && movement.is_none() {
                continue;
            }
            if !player.teleport_confirmed() {
                // Movement is untrusted until the entry teleport is acked.
                continue;
            }
            // The angles are applied first (Шаг 19a). When the movement packet
            // itself carries them it is the same packet and applying it twice
            // would only duplicate the work; when it is position-only, the
            // rotation slot holds the `move_player_rot` the client sent in the
            // same tick, which the movement slot would otherwise have dropped.
            if let Some(state) = rotation {
                if !movement
                    .as_ref()
                    .is_some_and(|newest| newest.kind.carries_rotation())
                {
                    match state.decode() {
                        Ok(update) => player.apply_rotation(&update),
                        Err(err) => warn!(
                            player = id,
                            kind = ?state.kind,
                            %err,
                            "dropping an undecodable rotation packet"
                        ),
                    }
                }
            }
            let Some(state) = movement else {
                continue;
            };
            match state.decode() {
                Ok(update) => {
                    player.apply_move(&update);
                    let position = player.position();
                    outcome.moves.push(AppliedMove {
                        region: self.id,
                        id,
                        seq,
                        position,
                    });
                    seq += 1;
                    let to = grid.region_of_position(position.x, position.z);
                    if to != self.id {
                        outcome.transfers.push(Transfer {
                            player: id,
                            from: self.id,
                            to,
                            seq,
                        });
                        seq += 1;
                    }
                    // The move may also have carried the player into another
                    // cache chunk: the window wants to move with it. One event
                    // per tick compares the centre at the tick's start with the
                    // final one, so several chunk steps in a tick still give
                    // one window difference (M4.4 B, ШАГ 2).
                    let center = chunk_of_position(position.x, position.z);
                    let previous = player.window_center;
                    if center != previous {
                        let (add, remove) =
                            window_difference(previous, center, player.view_distance());
                        outcome.streams.push(StreamChunks {
                            region: self.id,
                            player: id,
                            seq,
                            center,
                            add,
                            remove,
                        });
                        player.window_center = center;
                        seq += 1;
                    }
                }
                Err(err) => warn!(
                    player = id,
                    kind = ?state.kind,
                    %err,
                    "dropping an undecodable movement packet"
                ),
            }
        }
        outcome
    }

    /// Slows [`Region::tick`] down for the ordering measurement (tests only).
    #[cfg(test)]
    pub(crate) fn set_test_delay(&mut self, delay: Duration) {
        self.delay = delay;
    }

    /// Sends a message to one player without ever blocking the tick.
    ///
    /// `try_send` never waits. When the queue is full the client is too
    /// slow: the player is removed and a [`Kick`] is returned so the game
    /// thread can log it and let the connection close itself.
    pub fn deliver(&mut self, id: PlayerId, message: ServerMessage) -> Option<Kick> {
        let player = self.players.get(&id)?;
        match player.outbound().try_send(message) {
            Ok(()) => None,
            Err(TrySendError::Full(_)) => {
                player.kick_flag().raise(KickReason::SendQueueOverflow);
                self.players.remove(&id);
                Some(Kick {
                    region: self.id,
                    id,
                    reason: KickReason::SendQueueOverflow,
                })
            }
            // The connection is already gone; nothing to kick.
            Err(TrySendError::Closed(_)) => None,
        }
    }
}

/// Burns the given amount of CPU time with arithmetic the compiler cannot
/// elide: the W2 workload of the M4.6 benchmark (ARCH-M4 §5), the mechanism
/// scalability ceiling rather than game speed.
fn spin(workload: Duration) {
    let deadline = Instant::now() + workload;
    let mut acc: u64 = 0;
    while Instant::now() < deadline {
        for _ in 0..1000 {
            acc = acc
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            std::hint::black_box(&acc);
        }
    }
    std::hint::black_box(acc);
}
