//! Messages crossing the game-thread boundary (ARCH-M4 §0).
//!
//! Upward (`ToGame`) the reliable messages travel with `send().await`: a
//! stalled network task only stalls itself. Downward each connection has
//! its own bounded queue (`ServerMessage`); the tick never blocks on it
//! (see the kick policy in [`Region::deliver`](super::Region::deliver)).
//! Movement is not a message but the [`MoveSlot`] of the player: the newest
//! packet wins and the old one is dropped.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use protocol::Position;
use tokio::sync::mpsc;

use super::entities::{EntityMove, EntityRemove, EntitySpawn};

/// Position of a player inside the game thread.
///
/// It is also the entity id of Login (play) for the first player of a
/// server: the counter starts at one, so a single connection keeps the id
/// the join always announced.
pub type PlayerId = i32;

/// Connection → game thread.
///
/// Sent with `send().await`: the message is never lost, and only the
/// sending connection task waits when the queue is full.
#[derive(Debug)]
pub enum ToGame {
    /// A connection reached the play state and registers its player.
    Join(Box<PlayerJoin>),
    /// The client confirmed the server teleport that carried this id.
    TeleportAck {
        /// Player whose teleport was confirmed.
        id: PlayerId,
        /// Teleport id echoed by the client.
        teleport_id: i32,
    },
    /// The connection ended; the player leaves the world.
    Quit {
        /// Player that is leaving.
        id: PlayerId,
    },
}

/// Registration of a joining player: who they are and how to reach them.
#[derive(Debug)]
pub struct PlayerJoin {
    /// Player id (from the game thread's counter).
    pub id: PlayerId,
    /// Name of Login Start.
    pub name: String,
    /// View distance the client asked for, in chunks.
    pub view_distance: i32,
    /// Block the player joins at (the save's spawn).
    pub spawn: Position,
    /// Game mode the join announces (M4.5 Шаг 7.3); from
    /// [`GameConfig::default_game_mode`](super::GameConfig).
    pub game_mode: i32,
    /// How the game thread reaches this connection.
    pub connection: PlayerConnection,
}

/// The game thread's handle on one connection.
#[derive(Debug)]
pub struct PlayerConnection {
    /// Bounded queue of packets the connection must send. A full queue
    /// means the client is too slow (the kick policy acts on it).
    pub outbound: mpsc::Sender<ServerMessage>,
    /// Raised when the game thread kicks the connection.
    pub kick: Arc<KickFlag>,
    /// The movement slot the connection writes and the tick reads.
    pub last_move: Arc<MoveSlot>,
}

/// Game thread → connection.
#[derive(Debug)]
pub enum ServerMessage {
    /// A packet the connection encodes and sends now. M4.1's tick produces
    /// none yet; M4.4's streaming uses the same path.
    Send {
        /// Numeric packet id.
        id: u32,
        /// Payload (packet id excluded).
        payload: Vec<u8>,
    },
    /// The player's chunk window moved (M4.4 B, ШАГ 2): the tick's centre
    /// change for one player. The connection turns it into
    /// `set_chunk_cache_center`, `forget_level_chunk` and new chunk
    /// batches (Шаг 3). `add` and `remove` are the window difference around
    /// `center`, already in wire order.
    StreamChunks {
        /// New cache centre, in chunks.
        center: (i32, i32),
        /// Chunks entering the window, in window send order.
        add: Vec<(i32, i32)>,
        /// Chunks leaving the window, ascending `(x, z)` (the traced order).
        remove: Vec<(i32, i32)>,
    },
    /// The players a viewer starts or stops seeing, plus their movement
    /// (M4.5 Шаг 3). The connection turns each list into the entity packets
    /// (spawn order per the trace); delivery uses the same `try_send` policy
    /// as every other tick → connection message.
    StreamEntities {
        /// Entities to spawn, in ascending id.
        spawn: Vec<EntitySpawn>,
        /// Entities to remove, in ascending id.
        remove: Vec<EntityRemove>,
        /// Movement updates, in ascending id.
        moves: Vec<EntityMove>,
    },
}

/// Why the game thread closes a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KickReason {
    /// The connection's outbound queue stayed full: the client is slow.
    SendQueueOverflow,
}

impl KickReason {
    /// Stable text of the reason, for logs and (later) the kick packet.
    pub fn message(self) -> &'static str {
        match self {
            Self::SendQueueOverflow => "send queue overflow",
        }
    }

    /// Wire code stored in [`KickFlag`]; zero means "not kicked".
    fn code(self) -> u8 {
        match self {
            Self::SendQueueOverflow => 1,
        }
    }

    /// Inverse of [`KickReason::code`].
    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::SendQueueOverflow),
            _ => None,
        }
    }
}

/// Lock-free record of "this connection was kicked, for this reason".
///
/// The game thread raises it once and closes the queue; the connection
/// reads it when its queue reports the end. No lock is needed.
#[derive(Debug, Default)]
pub struct KickFlag(AtomicU8);

impl KickFlag {
    /// Raises the flag. Never clears it.
    pub fn raise(&self, reason: KickReason) {
        self.0.store(reason.code(), Ordering::Release);
    }

    /// The reason the connection was kicked, if it was.
    pub fn reason(&self) -> Option<KickReason> {
        KickReason::from_code(self.0.load(Ordering::Acquire))
    }
}

/// Which movement packet filled a [`MoveSlot`] entry (M4.2).
///
/// The connection already matched the numeric id against the runtime packet
/// ids, so it knows the kind; parsing the payload itself happens on the game
/// thread ([`MoveState::decode`](super::MoveState::decode)), keeping movement
/// semantics off the network task (ARCH-M4 §0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveKind {
    /// `minecraft:move_player_pos`: absolute position, no rotation.
    Pos,
    /// `minecraft:move_player_pos_rot`: absolute position and rotation.
    PosRot,
    /// `minecraft:move_player_rot`: rotation only.
    Rot,
    /// `minecraft:move_player_status_only`: the flags byte only.
    StatusOnly,
}

/// The newest movement packet of one player: its kind and raw payload.
///
/// M4.1 only stored the newest packet; M4.2 decodes it in the tick and
/// applies the position. Keeping the payload raw here means the connection
/// touches no movement semantics at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveState {
    /// Which movement packet this is.
    pub kind: MoveKind,
    /// Payload of the movement packet (packet id excluded).
    pub payload: Vec<u8>,
}

/// Everything the connection last saw for one player (M4.5 Шаг 8).
///
/// Movement keeps its "newest wins" rule; the input flags byte and the swing
/// events are independent slots, so a `player_input` arriving between two
/// movement packets cannot swallow the movement (and vice versa). `swings`
/// counts the `punch` packets since the last [`MoveSlot::take_swings`]; the
/// tick drains it and the count is the source for the future `animate`
/// broadcast (not written before the vanilla traces establish its bytes).
#[derive(Debug, Default)]
struct SlotValue {
    /// The newest movement packet, if any.
    movement: Option<MoveState>,
    /// The newest `player_input` flags byte, if any.
    input: Option<u8>,
    /// `punch` packets since the last drain.
    swings: u32,
}

/// The movement slot of one player: a single latest value per category, not
/// a queue.
///
/// The connection overwrites the movement and input entries as packets
/// arrive; the tick takes the newest value once per tick. The lock is
/// uncontended (one writer, one reader, a trivial copy) and is the
/// documented exception of M4.1 — see `docs/DECISIONS.md`. Step 8 added the
/// input and swing entries to the same slot rather than a second lock.
#[derive(Debug, Default)]
pub struct MoveSlot(Mutex<SlotValue>);

impl MoveSlot {
    /// Stores the newest movement packet, dropping whatever was unread.
    pub fn store(&self, state: MoveState) {
        let mut slot = self.lock();
        slot.movement = Some(state);
    }

    /// Stores the newest `player_input` flags byte (M4.5 Шаг 8).
    pub fn store_input(&self, flags: u8) {
        self.lock().input = Some(flags);
    }

    /// Records one `punch` (swing) event (M4.5 Шаг 8).
    pub fn store_swing(&self) {
        let mut slot = self.lock();
        slot.swings = slot.swings.saturating_add(1);
    }

    /// Takes the newest movement packet, leaving the slot empty.
    pub fn take(&self) -> Option<MoveState> {
        self.lock().movement.take()
    }

    /// Takes the newest input flags byte, leaving the entry empty.
    pub fn take_input(&self) -> Option<u8> {
        self.lock().input.take()
    }

    /// Takes and resets the swing count.
    pub fn take_swings(&self) -> u32 {
        std::mem::take(&mut self.lock().swings)
    }

    /// Locks the slot, recovering from a poisoned lock: the value is a
    /// plain copy and never observed half-updated.
    fn lock(&self) -> std::sync::MutexGuard<'_, SlotValue> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
