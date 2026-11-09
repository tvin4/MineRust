//! One region owns its players (ARCH-M4 §1).
//!
//! M4.1 runs exactly one region; the shape (an id, a player map, a
//! `tick`) is what M4.3 turns into a pool of regions, so the scheduler
//! never changes when a second region appears.

use std::collections::HashMap;
use std::time::Instant;

use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;

use super::messages::{
    KickFlag, KickReason, MoveState, PlayerConnection, PlayerId, PlayerJoin, ServerMessage,
};

/// Id of a region. M4.1 has exactly one, with the id in [`SINGLE_REGION`].
pub type RegionId = i32;

/// The only region of M4.1. M4.3 replaces it with a chunk → region map.
pub const SINGLE_REGION: RegionId = 0;

/// A player the game thread owns. M4.1 keeps enough to route messages and
/// to know whether movement may be trusted; positions are not applied yet.
#[derive(Debug)]
pub struct Player {
    id: PlayerId,
    name: String,
    view_distance: i32,
    teleport_confirmed: bool,
    spawn: protocol::Position,
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

    /// The connection's outbound queue.
    pub fn outbound(&self) -> &mpsc::Sender<ServerMessage> {
        &self.connection.outbound
    }

    /// The kick flag shared with the connection.
    pub fn kick_flag(&self) -> &KickFlag {
        &self.connection.kick
    }

    /// Takes the newest movement packet; M4.1 only drops it (M4.2 applies).
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

/// One region: its id and the players it owns.
#[derive(Debug)]
pub struct Region {
    id: RegionId,
    players: HashMap<PlayerId, Player>,
}

impl Region {
    /// A region with no players.
    pub fn new(id: RegionId) -> Self {
        Self {
            id,
            players: HashMap::new(),
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

    /// Ticks the region once.
    ///
    /// M4.1 has nothing to apply: the movement slots are drained and the
    /// values dropped (M4.2 applies them). No player is kicked here yet —
    /// kicks come from [`Region::deliver`], which M4.4's streaming calls.
    pub fn tick(&mut self, _tick: u64, _now: Instant) -> Vec<Kick> {
        for player in self.players.values() {
            let _ignored = player.take_move();
        }
        Vec::new()
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
