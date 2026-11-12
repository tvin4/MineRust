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

use super::entities::StreamEntities;
use super::grid::{chunk_of_position, RegionGrid};
use super::messages::{
    KickFlag, KickReason, MoveState, PlayerConnection, PlayerId, PlayerJoin, ServerMessage,
};
use super::movement::MoveUpdate;
use crate::chunks::window_difference;

/// Id of a region. M4.1 has exactly one, with the id in [`SINGLE_REGION`].
pub type RegionId = i32;

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
    /// The raw movement flags byte the client sent (bit 0: on the ground,
    /// [`PlayerPos::on_ground`]; other bits are preserved, not interpreted).
    pub flags: u8,
}

impl PlayerPos {
    /// The entry position: the centre of the spawn block, the same point the
    /// join teleport sends (`join.rs`).
    fn entry(spawn: protocol::Position) -> Self {
        Self {
            x: f64::from(spawn.x) + 0.5,
            y: f64::from(spawn.y),
            z: f64::from(spawn.z) + 0.5,
            yaw: 0.0,
            pitch: 0.0,
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
        }
        if let Some(pitch) = update.pitch {
            self.pitch = pitch;
        }
        self.flags = update.flags;
    }
}

/// A player the game thread owns: routing, whether movement may be trusted
/// and the newest applied position (M4.2).
#[derive(Debug)]
pub struct Player {
    id: PlayerId,
    name: String,
    view_distance: i32,
    teleport_confirmed: bool,
    spawn: protocol::Position,
    position: PlayerPos,
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
    connection: PlayerConnection,
}

impl Player {
    /// Builds a player from a join registration.
    pub fn new(join: PlayerJoin) -> Self {
        let position = PlayerPos::entry(join.spawn);
        let window_center = chunk_of_position(position.x, position.z);
        Self {
            id: join.id,
            name: join.name,
            view_distance: join.view_distance,
            teleport_confirmed: false,
            spawn: join.spawn,
            position,
            window_center,
            visible: BTreeMap::new(),
            last_broadcast: None,
            connection: join.connection,
        }
    }

    /// Player id.
    pub fn id(&self) -> PlayerId {
        self.id
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

    /// The pose last broadcast to this player's viewers, or `None`.
    pub fn last_broadcast(&self) -> Option<PlayerPos> {
        self.last_broadcast
    }

    /// Replaces the last broadcast pose (the tick's visibility pass owns it).
    pub fn set_last_broadcast(&mut self, pose: Option<PlayerPos>) {
        self.last_broadcast = pose;
    }

    /// Applies a decoded movement packet to this player.
    ///
    /// No movement is validated in M4 (ARCH-M4 §0): the values are accepted
    /// as they arrive.
    pub fn apply_move(&mut self, update: &MoveUpdate) {
        self.position.apply(update);
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
            let Some(state) = player.take_move() else {
                continue;
            };
            if !player.teleport_confirmed() {
                // Movement is untrusted until the entry teleport is acked.
                continue;
            }
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
