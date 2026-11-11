//! Bench players: the same `MoveSlot`/`outbound` path connections use, with
//! no network in between (ARCH-M4 §5).
//!
//! A bench player is a [`PlayerJoin`] whose `PlayerConnection` carries a real
//! outbound channel (nothing is sent to it without the engines delivering
//! the tick's own outcomes) and a real [`MoveSlot`] the bench writes move
//! packets into — exactly what the connection task writes during a live
//! session. The tick then decodes the payloads, applies positions and emits
//! window streams like always.

use std::sync::Arc;

use net::game::{
    KickFlag, MoveKind, MoveSlot, MoveState, PlayerConnection, PlayerId, PlayerJoin, ServerMessage,
    OUTBOUND_CAPACITY,
};
use tokio::sync::mpsc;

/// One bench player: the pieces to register it and the slot to feed.
pub struct BenchPlayer {
    /// The movement slot the connection would write; the bench writes here.
    pub moves: Arc<MoveSlot>,
    /// Position the player spawns at, in blocks.
    pub spawn: (i32, i32, i32),
    /// Block steps per tick the player walks (+x), `0` = stands still.
    pub step: f64,
    /// Current x position (f64, in blocks).
    x: f64,
    /// The join registration, taken by [`take_join`].
    join: Option<PlayerJoin>,
    /// Player id.
    id: PlayerId,
}

impl BenchPlayer {
    /// The player id this bench player was built with.
    pub fn id(&self) -> PlayerId {
        self.id
    }

    /// Takes the join registration (the scheduler consumes it).
    pub fn take_join(&mut self) -> PlayerJoin {
        self.join
            .take()
            .expect("each bench player registers exactly once")
    }

    /// Confirms the entry teleport flag a test/bench pre-set.
    pub(crate) fn set_step(&mut self, step: f64) {
        self.step = step;
    }
}

/// Creates `count` players spread over the grid so that `regions > 1` puts
/// them in several region tiles: players are laid out on a `√count`-wide
/// grid, spaced `spread` chunks apart, starting at chunk `(start)`.
///
/// With `regions == 1` (`side = 0`) the layout does not matter — everything
/// lands in [`net::game::SINGLE_REGION`] anyway.
pub fn spread(count: usize, spread_chunks: i32, start: (i32, i32)) -> Vec<BenchPlayer> {
    let side = (count as f64).sqrt().ceil().max(1.0) as i32;
    let mut players = Vec::with_capacity(count);
    for index in 0..count {
        let gx = start.0 + (index as i32 % side) * spread_chunks;
        let gz = start.1 + (index as i32 / side) * spread_chunks;
        let spawn = (gx * 16 + 8, 64, gz * 16 + 8);
        let (outbound, _rx) = mpsc::channel::<ServerMessage>(OUTBOUND_CAPACITY);
        let moves = Arc::new(MoveSlot::default());
        let id = index as i32 + 1;
        players.push(BenchPlayer {
            join: Some(PlayerJoin {
                id,
                name: format!("bench-{index}"),
                view_distance: 10,
                spawn: protocol::Position {
                    x: spawn.0,
                    y: spawn.1,
                    z: spawn.2,
                },
                connection: PlayerConnection {
                    outbound,
                    kick: Arc::new(KickFlag::default()),
                    last_move: Arc::clone(&moves),
                },
            }),
            moves,
            spawn,
            step: 0.0,
            x: f64::from(spawn.0),
            id,
        });
    }
    players
}

/// Sets the walking speed (blocks per tick, +x) of every player.
pub fn set_step(players: &mut [BenchPlayer], step: f64) {
    for player in players {
        player.set_step(step);
    }
}

/// Builds the `move_player_pos` payload (3 big-endian doubles + flags) the
/// connection would forward — the same bytes, so the tick decodes the same
/// path.
pub fn pos_move_payload(x: f64, y: f64, z: f64) -> MoveState {
    let mut payload = Vec::with_capacity(25);
    payload.extend_from_slice(&x.to_be_bytes());
    payload.extend_from_slice(&y.to_be_bytes());
    payload.extend_from_slice(&z.to_be_bytes());
    payload.push(0x01); // on ground
    MoveState {
        kind: MoveKind::Pos,
        payload,
    }
}

/// Advances every walking player one step and stores its move packet.
pub fn tick_moves(players: &mut [BenchPlayer]) {
    for player in players {
        if player.step == 0.0 {
            continue;
        }
        player.x += player.step;
        player
            .moves
            .store(pos_move_payload(player.x, 64.0, f64::from(player.spawn.2)));
    }
}

/// Inserts every player into the scheduler (joins are consumed) and confirms
/// their entry teleports, so moves are applied (the tick drops movement until
/// then, ARCH-M4 §0).
pub fn register_all(scheduler: &mut net::game::Scheduler, players: &mut [BenchPlayer]) {
    let joins: Vec<net::game::PlayerJoin> = players.iter_mut().map(|p| p.take_join()).collect();
    for join in joins {
        scheduler.insert_player(join);
    }
    for player in players {
        scheduler.confirm_teleport(player.id());
    }
}
