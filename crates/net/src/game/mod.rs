//! The game thread: one 20 TPS tick that owns the players (ARCH-M4 §0/§1).
//!
//! M4.1 runs a single region on a dedicated `std::thread`. Connections
//! stay thin: they decode packets, keep keep-alives and hand the world's
//! chunk batches to `batch.rs`; everything that is game state — who is
//! online, whether their teleport is confirmed, the newest movement —
//! lives here and is only reached through channels.
//!
//! The tick waits for an absolute deadline (`start + i·50 ms`), reads the
//! incoming channel with `try_recv` and never waits on a connection. Axis
//! choices, thresholds and the replay format are M4.1 **[hypothesis]**
//! where no trace backs them (see `docs/ARCH-M4.md` §0/§1/§3).

mod messages;
mod metrics;
mod movement;
mod region;
mod replay;
mod scheduler;

#[cfg(test)]
mod tests;

pub use messages::{
    KickFlag, KickReason, MoveKind, MoveSlot, MoveState, PlayerConnection, PlayerId, PlayerJoin,
    ServerMessage, ToGame,
};
pub use metrics::{TickMetrics, TickMetricsHandle};
pub use movement::MoveUpdate;
pub use region::{
    AppliedMove, Kick, Player, PlayerPos, Region, RegionId, TickOutcome, SINGLE_REGION,
};
pub use replay::{Replay, ReplayRecord};
pub use scheduler::Scheduler;

use std::io;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tracing::{info, warn};

use metrics::Recorder;

/// One game tick: 20 TPS.
pub const TICK_PERIOD: Duration = Duration::from_millis(50);

/// How often the tick metrics are logged.
const LOG_EVERY: Duration = Duration::from_secs(5);

/// Environment variable that turns the replay journal on; its value is the
/// path of the JSONL file (ARCH-M4 §1).
const REPLAY_ENV: &str = "MINERUST_REPLAY";

/// Default capacity of the connection → game channel.
const JOIN_CAPACITY: usize = 256;

/// Default capacity of each connection's outbound queue.
pub const OUTBOUND_CAPACITY: usize = 256;

/// Configuration of the game thread.
///
/// Every field is a plain scalar so M4.6's benchmark can shape the thread
/// without touching the loop. `reset_after` is the "[hypothesis] N ticks"
/// of ARCH-M4 §3 (reset the deadline when the tick falls this far behind);
/// `ticks_limit` stops the loop after a fixed count (bench and tests).
#[derive(Debug, Clone, Copy)]
pub struct GameConfig {
    /// Period of one tick.
    pub period: Duration,
    /// Reset the deadline when the tick is this many periods behind.
    pub reset_after: u32,
    /// Capacity of the connection → game channel.
    pub join_capacity: usize,
    /// Capacity of each connection's outbound queue.
    pub outbound_capacity: usize,
    /// Stop after this many ticks; `None` runs until the channel closes.
    pub ticks_limit: Option<u64>,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            period: TICK_PERIOD,
            reset_after: 5,
            join_capacity: JOIN_CAPACITY,
            outbound_capacity: OUTBOUND_CAPACITY,
            ticks_limit: None,
        }
    }
}

/// The game thread is gone; a message could not be delivered.
#[derive(Debug, thiserror::Error)]
#[error("the game thread is not running")]
pub struct GameClosed;

/// The spawned game thread. Keep it to `join` or `join_timeout`; the server
/// joins it with a timeout on shutdown.
#[derive(Debug)]
pub struct GameThread {
    join: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl GameThread {
    /// Waits for the loop to finish on its own (a tick limit or a closed
    /// channel); does not ask it to stop.
    pub fn join(mut self) -> std::thread::Result<()> {
        match self.join.take() {
            Some(join) => join.join(),
            None => Ok(()),
        }
    }

    /// Asks the loop to stop and waits at most `timeout` for it to finish.
    ///
    /// The loop leaves after its current tick (the stop flag is read once per
    /// tick), so the wait is short. Returns [`io::ErrorKind::TimedOut`] when
    /// the thread is still running at the deadline; the caller may then exit
    /// anyway instead of hanging.
    pub fn join_timeout(mut self, timeout: Duration) -> io::Result<()> {
        self.stop.store(true, Ordering::Release);
        let join = match self.join.take() {
            Some(join) => join,
            None => return Ok(()),
        };
        let deadline = Instant::now() + timeout;
        while !join.is_finished() {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the game thread did not stop within the timeout",
                ));
            }
            std::thread::sleep(STOP_POLL);
        }
        join.join()
            .map_err(|_| io::Error::other("the game thread panicked"))
    }
}

/// How often [`GameThread::join_timeout`] checks whether the loop has left.
const STOP_POLL: Duration = Duration::from_millis(5);

/// A cloneable handle on the game thread, held by every connection.
#[derive(Clone)]
pub struct GameHandle {
    to_game: mpsc::Sender<ToGame>,
    next_id: Arc<AtomicI32>,
    metrics: Arc<TickMetricsHandle>,
    stop: Arc<AtomicBool>,
}

impl GameHandle {
    /// Starts the game thread.
    ///
    /// The id counter starts at one, so the first player of a server keeps
    /// the entity id the join sequence always announced.
    pub fn spawn(config: GameConfig) -> io::Result<(GameThread, Self)> {
        let capacity = config.join_capacity.max(1);
        let (to_game, incoming) = mpsc::channel(capacity);
        let metrics = Arc::new(TickMetricsHandle::default());
        let thread_metrics = Arc::clone(&metrics);
        let stop = Arc::new(AtomicBool::new(false));
        let loop_stop = Arc::clone(&stop);
        let join = std::thread::Builder::new()
            .name("minecraft-game".to_owned())
            .spawn(move || game_loop(incoming, config, &thread_metrics, &loop_stop))?;
        let handle = Self {
            to_game,
            next_id: Arc::new(AtomicI32::new(1)),
            metrics,
            stop: Arc::clone(&stop),
        };
        Ok((
            GameThread {
                join: Some(join),
                stop,
            },
            handle,
        ))
    }

    /// The next free player id.
    pub fn next_player_id(&self) -> PlayerId {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// The last published tick metrics.
    pub fn metrics(&self) -> TickMetrics {
        self.metrics.snapshot()
    }

    /// The metrics handle, so a reader can outlive the handle clones.
    pub fn metrics_handle(&self) -> Arc<TickMetricsHandle> {
        Arc::clone(&self.metrics)
    }

    /// Delivers a reliable message; waits only when the queue is full.
    pub async fn send(&self, message: ToGame) -> Result<(), GameClosed> {
        self.to_game.send(message).await.map_err(|_| GameClosed)
    }

    /// Best-effort quit that never waits, for the moment a connection drops.
    pub fn quit_now(&self, id: PlayerId) {
        let _ = self.to_game.try_send(ToGame::Quit { id });
    }

    /// Asks the game thread to leave after its current tick.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// The tick loop: wait for the absolute deadline, apply incoming messages,
/// tick the regions, publish the metrics, repeat.
///
/// The loop leaves when the incoming channel closes (every [`GameHandle`]
/// dropped) or when [`GameHandle::stop`] raised the shared flag; the stop
/// flag is read once per tick, so a shutdown waits at most one period.
fn game_loop(
    mut incoming: mpsc::Receiver<ToGame>,
    config: GameConfig,
    metrics: &TickMetricsHandle,
    stop: &AtomicBool,
) {
    let mut scheduler = Scheduler::new(vec![Region::new(SINGLE_REGION)]);
    let mut recorder = Recorder::new(config.period);
    let mut replay = Replay::new(open_replay());
    let started = Instant::now();
    let mut deadline = started + config.period;
    let mut last_log = started;
    let mut tick: u64 = 0;
    let mut disconnected = false;
    loop {
        sleep_until(deadline);
        if stop.load(Ordering::Acquire) {
            info!(ticks = tick, "stop requested, leaving the tick loop");
            break;
        }
        let woke = Instant::now();
        let late = woke.saturating_duration_since(deadline);
        if late > Duration::from_millis(metrics::LATE_TICK_MS) {
            warn!(
                tick,
                late_ms = late.as_secs_f64() * 1000.0,
                uptime_ms = started.elapsed().as_secs_f64() * 1000.0,
                players = scheduler.player_count(),
                "tick woke late"
            );
        }
        let work = Instant::now();
        apply_queued(
            &mut scheduler,
            &mut replay,
            tick,
            &mut incoming,
            &mut disconnected,
        );
        let outcome = scheduler.tick(tick, deadline);
        // Positions the tick applied (M4.2) are recorded after the queued
        // messages, so `seq` continues the per-tick counter that
        // `apply_queued` opened.
        record_moves(&mut replay, tick, &outcome.moves);
        for kick in &outcome.kicks {
            warn!(
                player = kick.id,
                reason = kick.reason.message(),
                "kicking a slow player"
            );
        }
        recorder.record(tick, deadline, woke, work.elapsed(), metrics);
        tick += 1;
        deadline += config.period;
        if config.ticks_limit.is_some_and(|limit| tick >= limit) {
            break;
        }
        let now = Instant::now();
        if let Some(behind) = overdue_by(now, deadline, &config) {
            warn!(
                behind_ms = behind.as_secs_f64() * 1000.0,
                "tick fell behind, resetting the deadline"
            );
            deadline = now + config.period;
        }
        if now.saturating_duration_since(last_log) >= LOG_EVERY {
            last_log = now;
            info!(ticks = tick, metrics = ?metrics.snapshot(), "game tick");
        }
        if disconnected {
            break;
        }
    }
    recorder.finish(metrics);
    if let Err(err) = replay.flush() {
        warn!(%err, "could not flush the replay journal");
    }
    info!(ticks = tick, metrics = ?metrics.snapshot(), "game thread stopped");
}

/// Opens the replay journal named by [`REPLAY_ENV`], if the variable is set.
fn open_replay() -> Option<std::fs::File> {
    let path = std::env::var_os(REPLAY_ENV)?;
    if path.is_empty() {
        return None;
    }
    let path = std::path::PathBuf::from(path);
    match std::fs::File::create(&path) {
        Ok(file) => {
            info!(path = %path.display(), "replay journal enabled");
            Some(file)
        }
        Err(err) => {
            warn!(%err, path = %path.display(), "could not open the replay journal");
            None
        }
    }
}

/// Pops one incoming message without waiting; `Err(Disconnected)` sets the
/// flag and yields `None`.
fn next_message(incoming: &mut mpsc::Receiver<ToGame>, disconnected: &mut bool) -> Option<ToGame> {
    match incoming.try_recv() {
        Ok(message) => Some(message),
        Err(mpsc::error::TryRecvError::Empty) => None,
        Err(mpsc::error::TryRecvError::Disconnected) => {
            *disconnected = true;
            None
        }
    }
}

/// Applies everything queued for one tick, recording each applied event.
///
/// The tick number is fixed for the whole batch and the replay order counter
/// restarts at zero, so `(tick, seq)` is the deterministic position of the
/// event inside the run (ARCH-M4 §1).
fn apply_queued<W: std::io::Write>(
    scheduler: &mut Scheduler,
    replay: &mut Replay<W>,
    tick: u64,
    incoming: &mut mpsc::Receiver<ToGame>,
    disconnected: &mut bool,
) {
    replay.start_tick();
    while let Some(message) = next_message(incoming, disconnected) {
        apply(scheduler, replay, tick, message);
    }
}

/// Applies one incoming message to the single region and records it.
fn apply<W: std::io::Write>(
    scheduler: &mut Scheduler,
    replay: &mut Replay<W>,
    tick: u64,
    message: ToGame,
) {
    match message {
        ToGame::Join(join) => {
            let payload = serde_json::json!({
                "name": &join.name,
                "view_distance": join.view_distance,
                "spawn": { "x": join.spawn.x, "y": join.spawn.y, "z": join.spawn.z },
            });
            replay.record(tick, Some(SINGLE_REGION), join.id, "join", payload);
            // M4.3 routes joins by chunk; M4.1 puts everyone in one region.
            match scheduler.region_mut(SINGLE_REGION) {
                Some(region) => {
                    if region.insert(*join).is_some() {
                        warn!("a join reused an id that was still online");
                    }
                }
                None => warn!("a join arrived for a missing region"),
            }
        }
        ToGame::TeleportAck { id, teleport_id } => {
            let region_id = scheduler.region_of(id);
            replay.record(
                tick,
                region_id,
                id,
                "teleport_ack",
                serde_json::json!({ "teleport_id": teleport_id }),
            );
            match region_id.and_then(|region_id| scheduler.region_mut(region_id)) {
                Some(region) => match region.get_mut(id) {
                    Some(player) => player.confirm_teleport(),
                    None => warn!(player = id, "teleport ack for an unknown player"),
                },
                None => warn!(player = id, "teleport ack for an unknown region"),
            }
            info!(player = id, teleport_id, "teleport confirmed");
        }
        ToGame::Quit { id } => {
            let region_id = scheduler.region_of(id);
            replay.record(tick, region_id, id, "quit", serde_json::json!({}));
            if let Some(region) = region_id.and_then(|region_id| scheduler.region_mut(region_id)) {
                region.remove(id);
            }
        }
    }
}

/// Records the positions one tick applied to the replay journal (M4.2).
///
/// Shared by the live loop and the replay test so both write the same
/// `move` line; `tick` is the tick the positions were applied in.
fn record_moves<W: std::io::Write>(replay: &mut Replay<W>, tick: u64, moves: &[AppliedMove]) {
    for applied in moves {
        replay.record(
            tick,
            Some(applied.region),
            applied.id,
            "move",
            serde_json::json!({
                "x": applied.position.x,
                "y": applied.position.y,
                "z": applied.position.z,
                "yaw": applied.position.yaw,
                "pitch": applied.position.pitch,
                "on_ground": applied.position.on_ground(),
                "flags": applied.position.flags,
            }),
        );
    }
}

/// Sleeps until `deadline`; returns at once when it has already passed.
fn sleep_until(deadline: Instant) {
    let now = Instant::now();
    if deadline > now {
        std::thread::sleep(deadline - now);
    }
}

/// How far a tick must fall behind before it gives up on catching up.
///
/// The loop never replays missed slots: when the deadline lags by more than
/// `reset_after` periods it resets once to "now + period" instead of firing
/// a burst of catch-up ticks (ARCH-M4 §3). Returns the lag when a reset is
/// due, `None` while the deadline is still within reach.
#[must_use]
fn overdue_by(now: Instant, deadline: Instant, config: &GameConfig) -> Option<Duration> {
    let behind = now.saturating_duration_since(deadline);
    (behind > config.period * config.reset_after).then_some(behind)
}
