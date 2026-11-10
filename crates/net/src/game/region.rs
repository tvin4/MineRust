//! One region owns its players (ARCH-M4 §1).
//!
//! M4.1 runs exactly one region; the shape (an id, a player map, a
//! `tick`) is what M4.3 turns into a pool of regions, so the scheduler
//! never changes when a second region appears.

use std::collections::HashMap;
use std::time::Instant;

#[cfg(test)]
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tracing::warn;

use super::messages::{
    KickFlag, KickReason, MoveState, PlayerConnection, PlayerId, PlayerJoin, ServerMessage,
};
use super::movement::MoveUpdate;

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
    connection: PlayerConnection,
}

impl Player {
    /// Builds a player from a join registration.
    pub fn new(join: PlayerJoin) -> Self {
        Self {
            id: join.id,
            name: join.name,
            view_distance: join.view_distance,
            teleport_confirmed: false,
            spawn: join.spawn,
            position: PlayerPos::entry(join.spawn),
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
    /// Player that was removed.
    pub id: PlayerId,
    /// Reason it was removed.
    pub reason: KickReason,
}

/// A position a region applied from a movement packet this tick (M4.2).
///
/// The tick returns these instead of writing the replay itself, so the
/// region stays generic over the journal sink and the game loop owns the
/// recording order (`(tick, seq)` is per event, ARCH-M4 §1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AppliedMove {
    /// Region that applied the move.
    pub region: RegionId,
    /// Player whose position moved.
    pub id: PlayerId,
    /// The player's position after the move.
    pub position: PlayerPos,
}

/// Everything the regions produced in one tick.
#[derive(Debug, Default, PartialEq)]
pub struct TickOutcome {
    /// Positions applied from movement packets, in region then player order.
    pub moves: Vec<AppliedMove>,
    /// Players kicked this tick.
    pub kicks: Vec<Kick>,
}

/// One region: its id and the players it owns.
#[derive(Debug)]
pub struct Region {
    id: RegionId,
    players: HashMap<PlayerId, Player>,
    /// Test-only slowdown of [`Region::tick`] (ordering measurement of the
    /// pool); zero in production.
    #[cfg(test)]
    delay: Duration,
}

impl Region {
    /// A region with no players.
    pub fn new(id: RegionId) -> Self {
        Self {
            id,
            players: HashMap::new(),
            #[cfg(test)]
            delay: Duration::ZERO,
        }
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

    /// Adds a player; returns the one already under that id, if any.
    pub fn insert(&mut self, join: PlayerJoin) -> Option<Player> {
        self.players.insert(join.id, Player::new(join))
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

    /// Ticks the region once: applies the newest movement of every player.
    ///
    /// A movement packet is applied only after the client confirmed the
    /// entry teleport ([`Player::teleport_confirmed`]); until then it is
    /// dropped (ARCH-M4 §0). No movement is validated in M4. Kicks are not
    /// produced here — they come from [`Region::deliver`], which M4.4's
    /// streaming calls.
    ///
    /// Players are visited in id order: a `HashMap` iteration order is not
    /// deterministic, and the pool's merge must be reproducible.
    pub fn tick(&mut self, _tick: u64, _now: Instant) -> TickOutcome {
        #[cfg(test)]
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        let mut outcome = TickOutcome::default();
        let mut ids: Vec<PlayerId> = self.players.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            let Some(player) = self.players.get_mut(&id) else {
                continue;
            };
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
                    outcome.moves.push(AppliedMove {
                        region: self.id,
                        id: player.id(),
                        position: player.position(),
                    });
                }
                Err(err) => warn!(
                    player = player.id(),
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
                    id,
                    reason: KickReason::SendQueueOverflow,
                })
            }
            // The connection is already gone; nothing to kick.
            Err(TrySendError::Closed(_)) => None,
        }
    }
}
