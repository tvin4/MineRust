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
    /// The outbound queue's receiving end, kept so a delivery measurement
    /// (`try_send`) meets a live queue: a dropped receiver makes every send
    /// fail with `Closed` at once and hides the channel cost.
    rx: mpsc::Receiver<ServerMessage>,
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

    /// Empties the outbound queue, so a tick's delivery does not fill it and
    /// kick the player (M4.5 Шаг 3.5, phase (в)).
    pub fn drain_outbound(&mut self) {
        while self.rx.try_recv().is_ok() {}
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
        players.push(bench_player(index, gx, gz));
    }
    players
}

/// Creates `count` players laid out so that **exactly** `tiles_per_axis²`
/// region tiles of `region_side` chunks own at least one player: player `i`
/// sits at the centre chunk of tile `i % tiles`, cycling over the tiles.
///
/// This is the W2 control (ARCH-M4 §5, BENCH-M4 §2): the earlier `spread`
/// with `spread_chunks = 8` and `region_side = 64` put every player in one
/// tile, so the pool never engaged (measured 2026-11: 1 region, inline at
/// any K). Placement at a tile's centre keeps a standing player inside it,
/// so the number of regions with players stays constant for the whole run.
pub fn spread_over_tiles(
    count: usize,
    region_side: i32,
    tiles_per_axis: i32,
    start_chunk: (i32, i32),
) -> Vec<BenchPlayer> {
    let axis = tiles_per_axis.max(1);
    let tiles = (axis * axis) as usize;
    let half = (region_side / 2).max(0);
    let mut players = Vec::with_capacity(count);
    for index in 0..count {
        let tile = (index % tiles) as i32;
        let gx = start_chunk.0 + (tile % axis) * region_side + half;
        let gz = start_chunk.1 + (tile / axis) * region_side + half;
        players.push(bench_player(index, gx, gz));
    }
    players
}

/// One bench player whose chunk is `(chunk_x, chunk_z)`; the shared
/// construction of every layout ([`spread`], [`spread_over_tiles`]).
fn bench_player(index: usize, chunk_x: i32, chunk_z: i32) -> BenchPlayer {
    bench_player_at(index, chunk_x * 16 + 8, 64, chunk_z * 16 + 8)
}

/// One bench player at an explicit block position (M4.5 Шаг 3.5 layouts).
fn bench_player_at(index: usize, x: i32, y: i32, z: i32) -> BenchPlayer {
    let (outbound, rx) = mpsc::channel::<ServerMessage>(OUTBOUND_CAPACITY);
    let moves = Arc::new(MoveSlot::default());
    let id = index as i32 + 1;
    BenchPlayer {
        join: Some(PlayerJoin {
            id,
            name: format!("bench-{index}"),
            view_distance: 10,
            spawn: protocol::Position { x, y, z },
            game_mode: 0,
            connection: PlayerConnection {
                outbound,
                kick: Arc::new(KickFlag::default()),
                last_move: Arc::clone(&moves),
            },
        }),
        moves,
        spawn: (x, y, z),
        step: 0.0,
        rx,
        x: f64::from(x),
        id,
    }
}

/// A compact square grid of `count` players one block apart (M4.5 Шаг 3.5,
/// **dense** geometry): the whole cluster spans at most 31 blocks, so every
/// player is inside every other's 160-block view radius — the visible set is
/// all `P − 1` others.
pub fn dense_cluster(count: usize) -> Vec<BenchPlayer> {
    let side = (count as f64).sqrt().ceil().max(1.0) as i32;
    (0..count)
        .map(|index| {
            let x = index as i32 % side;
            let z = index as i32 / side;
            bench_player_at(index, x, 64, z)
        })
        .collect()
}

/// `count` players on a square grid spanning `span` blocks (M4.5 Шаг 3.5,
/// **uniform** geometry): nominal density `count / span²`, so a player sees
/// about `count · π · 160² / span²` neighbours — about 16 at `P = 200` over
/// `span = 1000`.
pub fn uniform_area(count: usize, span: i32) -> Vec<BenchPlayer> {
    let side = (count as f64).sqrt().ceil().max(1.0) as i32;
    let step = (span / side).max(1);
    (0..count)
        .map(|index| {
            let x = (index as i32 % side) * step;
            let z = (index as i32 / side) * step;
            bench_player_at(index, x, 64, z)
        })
        .collect()
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
