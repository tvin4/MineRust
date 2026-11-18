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

mod block_action;
mod break_block;
mod dig;
mod entities;
mod grid;
mod messages;
mod metrics;
mod movement;
mod playerdata;
mod pool;
mod region;
mod replay;
mod scheduler;
mod worldsave;

#[cfg(test)]
mod block_action_tests;
#[cfg(test)]
mod dig_oracle_tests;
#[cfg(test)]
mod entities_equivalence_tests;
#[cfg(test)]
mod entities_reference;
#[cfg(test)]
mod entities_tests;
#[cfg(test)]
mod region_tests;
#[cfg(test)]
mod replay_tests;
#[cfg(test)]
mod s10_fixture;
#[cfg(test)]
mod s11_fixture;
#[cfg(test)]
mod state_tests;
#[cfg(test)]
mod step4_tests;
#[cfg(test)]
mod swim_tests;
#[cfg(test)]
mod swing_live_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

pub use entities::{
    byte_angle, is_swimming, pitch_byte, pose_and_flags, pose_of, relative_delta, state_flags,
    view_radius, EntityMotion, EntityMove, EntityRemove, EntitySpawn, EntityStateUpdate, Forced,
    StreamEntities, BLOCKS_PER_CHUNK, CRAWLING_HEIGHT, CROUCHING_HEIGHT, PLAYER_ENTITY_TYPE,
    STANDING_HEIGHT,
};
pub use grid::{chunk_of_position, RegionGrid, CHUNK_SIZE};
pub use messages::{
    BlockAction, KickFlag, KickReason, MoveKind, MoveSlot, MoveState, PlayerConnection, PlayerId,
    PlayerJoin, ServerMessage, ToGame,
};
pub use metrics::{TickMetrics, TickMetricsHandle};
pub use movement::MoveUpdate;
pub use playerdata::PlayerdataRuntime;
pub use region::{
    AppliedMove, DigState, Kick, Player, PlayerPos, Region, RegionId, StreamChunks, TickOutcome,
    Transfer, SINGLE_REGION, SWING_DURATION_TICKS, SWING_THRESHOLD_TICKS,
};
pub use replay::{Replay, ReplayRecord};
pub use scheduler::Scheduler;
pub use worldsave::{WorldSave, SAVE_CHUNKS_PER_PASS};

use std::io;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tracing::{info, warn};

use self::playerdata::{save_all, save_player, stage_all};
use metrics::Recorder;

use crate::store::ChunkStore;

/// One game tick: 20 TPS.
pub const TICK_PERIOD: Duration = Duration::from_millis(50);

/// How often the tick metrics are logged.
const LOG_EVERY: Duration = Duration::from_secs(5);

/// A tick slower than this is reported with its phase breakdown (M4.5 step
/// 8). Far above the period, so a healthy tick pays only a few `Instant`
/// reads and no formatting.
const PHASE_LOG_MS: u64 = 50;

/// How long each stage of one tick took (M4.5 step 8).
///
/// The stages follow the tick loop: applying the queued messages, the
/// regions' own work, recording the replay, delivering the chunk streams and
/// delivering the entity streams. The sums are the tick's work between the
/// wake-up and the end of delivery; comparing them with `wake_late_ms` says
/// whether a slow tick stalled outside or inside the loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct TickPhases {
    /// `apply_queued`: draining the game-thread channel.
    apply_us: u64,
    /// `scheduler.tick`: every region's movement/visibility work.
    sched_us: u64,
    /// `record_moves` + `record_transfers`: the replay journal.
    moves_us: u64,
    /// `deliver_streams`: chunk-window events to the connections.
    streams_us: u64,
    /// `deliver_entities`: spawn/motion events to the connections.
    entities_us: u64,
}

impl TickPhases {
    /// The sum of the recorded phases, in microseconds.
    fn total_us(&self) -> u64 {
        self.apply_us + self.sched_us + self.moves_us + self.streams_us + self.entities_us
    }
}

/// Converts a phase duration to whole microseconds (the warn field's unit).
fn phase_us(took: Duration) -> u64 {
    u64::try_from(took.as_micros()).unwrap_or(u64::MAX)
}

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
///
/// `store` is the one non-scalar field (Шаг 13b): the shared wire world the
/// game thread changes blocks in. It is `None` for every caller without a
/// ready world (tests, the bench), and a block action is then acknowledged
/// without changing anything.
#[derive(Debug, Clone)]
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
    /// Worker threads of the region pool (M4.3); `0` means no pool and a
    /// sequential tick, which is what every pre-M4.3 caller keeps using.
    pub workers: usize,
    /// Side of a region tile in chunks (M4.3 part B); `0` means no split:
    /// the whole world is [`SINGLE_REGION`], the pre-M4.3 behaviour.
    pub region_side: i32,
    /// Raise the game thread's priority one step above normal (A3, the
    /// M4.6 priority question). Windows only
    /// (`THREAD_PRIORITY_ABOVE_NORMAL`); other platforms ignore the flag.
    /// Default off: the measured difference is judged in `docs/BENCH-M4.md`
    /// before this is ever recommended.
    pub tick_thread_priority: bool,
    /// Game mode every joining player gets (M4.5 Шаг 7.3): `0` survival,
    /// `1` creative, `2` adventure, `3` spectator. It is what the login
    /// `game_mode` and the tab-list `player_info_update` announce, and what
    /// selects the mode-dependent `set_entity_data` bytes. Only `0..=3` is
    /// accepted; [`GameHandle::spawn`] rejects anything else at startup.
    /// Default `0` — the real server keeps working exactly as before. Mode
    /// `3` is not fully supported (no `player_abilities`, no spectator
    /// flight), see `docs/DECISIONS.md`.
    pub default_game_mode: i32,
    /// The shared wire world the game thread changes blocks in (Шаг 13b).
    /// `None` means no world is attached and a `player_action` only gets its
    /// `block_changed_ack`.
    pub store: Option<Arc<ChunkStore>>,
    /// Hardness and bare-hand dig speed of every block state (Шаг 16a), read
    /// from the optional `hardness_report`. `None` means no report was loaded,
    /// so the mining timer has nothing to measure a dig against and keeps
    /// trusting the client about when a block is broken.
    pub hardness: Option<Arc<data::BlockHardness>>,
    /// Finish-time tolerance of the mining timer (Шаг 16a.3): a survival
    /// finish is accepted once the server's progress is within this many ticks
    /// of the expected `1.0`; `None` keeps the stage broadcasts but trusts
    /// every finish.
    ///
    /// The default is `None` (Шаг 18.0.1, owner's decision): the parity stand
    /// of 17b showed the vanilla 26.3 server **accepts** a finish one client
    /// tick into blocks as hard as `ender_chest` (`docs/BLOCK-BREAKING.md`
    /// §1.11), so our `Some(3)` was stricter than vanilla and rolled back an
    /// honest dig (the `_0131` run, where the client's tick catch-up
    /// compressed a 6-tick dig into 2.65 server ticks). `Some(n)` stays
    /// available as a setting; the data is one stand, so this is temporary.
    pub dig_tolerance_ticks: Option<u32>,
    /// Player save runtime (Шаг 18a). `None` keeps every player in memory
    /// only: nothing is written and every join starts at the spawn with
    /// fresh vitals. The connection reads the store at join and the game
    /// thread queues snapshots for the writer.
    pub playerdata: Option<Arc<PlayerdataRuntime>>,
    /// World-save writer (Шаг 22b). `None` keeps the world in memory only:
    /// nothing is written and a restart starts from the template.
    pub worldsave: Option<Arc<dyn WorldSave>>,
    /// Autosave period of the world in ticks (Шаг 22b); `0` disables the
    /// periodic pass. The shutdown save still runs.
    pub world_autosave_ticks: u64,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            period: TICK_PERIOD,
            reset_after: 5,
            join_capacity: JOIN_CAPACITY,
            outbound_capacity: OUTBOUND_CAPACITY,
            ticks_limit: None,
            workers: 0,
            region_side: 0,
            tick_thread_priority: false,
            default_game_mode: 0,
            store: None,
            hardness: None,
            dig_tolerance_ticks: None,
            playerdata: None,
            worldsave: None,
            world_autosave_ticks: 0,
        }
    }
}

/// Raises the calling thread's priority one step above normal (A3). Windows
/// only: `SetThreadPriority` with `THREAD_PRIORITY_ABOVE_NORMAL` (+1). A
/// failure is logged, never fatal — the tick loop is correct either way.
// SOURCE: Microsoft Win32 `SetThreadPriority` / `THREAD_PRIORITY_ABOVE_NORMAL`
// documentation (learn.microsoft.com); the effect is what the A3 A/B series
// measures, the call itself is the documented minimal form.
#[cfg(windows)]
fn raise_game_thread_priority() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetThreadPriority(hthread: *mut core::ffi::c_void, npriority: i32) -> i32;
        fn GetCurrentThread() -> *mut core::ffi::c_void;
    }
    const THREAD_PRIORITY_ABOVE_NORMAL: i32 = 1;
    // SAFETY: SetThreadPriority on the current thread's pseudo-handle with a
    // documented constant; it cannot move memory or unwind. A dedicated test
    // calls this once (see tests::priority_raise_does_not_panic).
    let ok = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL) };
    if ok == 0 {
        warn!("tick thread priority raise failed (GetLastError ignored)");
    } else {
        info!("tick thread priority raised above normal");
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
    /// Game mode the join announces (M4.5 Шаг 7.3); validated at spawn.
    default_game_mode: i32,
    /// Player save runtime (Шаг 18a): read at join, kept so the connection
    /// sees the same store the writer thread was started with.
    playerdata: Option<Arc<PlayerdataRuntime>>,
}

impl GameHandle {
    /// Starts the game thread.
    ///
    /// The id counter starts at one, so the first player of a server keeps
    /// the entity id the join sequence always announced.
    pub fn spawn(config: GameConfig) -> io::Result<(GameThread, Self)> {
        Self::spawn_with_metrics(config, None)
    }

    /// [`Self::spawn`] with a metrics handle the caller owns (M4.4 B): the
    /// in-process parity walk reads the tick percentiles of its own run
    /// through the very handle the game thread publishes into. `None` keeps
    /// the private handle [`Self::spawn`] always used.
    pub fn spawn_with_metrics(
        config: GameConfig,
        metrics: Option<Arc<TickMetricsHandle>>,
    ) -> io::Result<(GameThread, Self)> {
        if !(0..=3).contains(&config.default_game_mode) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "default_game_mode {} is out of range: only 0..=3 \
                     (survival, creative, adventure, spectator) is accepted",
                    config.default_game_mode
                ),
            ));
        }
        let capacity = config.join_capacity.max(1);
        let (to_game, incoming) = mpsc::channel(capacity);
        let metrics = metrics.unwrap_or_default();
        let thread_metrics = Arc::clone(&metrics);
        let stop = Arc::new(AtomicBool::new(false));
        let loop_stop = Arc::clone(&stop);
        let priority = config.tick_thread_priority;
        // `config` is no longer `Copy` (Шаг 13b: it owns the world store), so
        // the fields the handle needs are read before it moves into the loop.
        let default_game_mode = config.default_game_mode;
        let playerdata = config.playerdata.clone();
        let join = std::thread::Builder::new()
            .name("minecraft-game".to_owned())
            .spawn(move || {
                if priority {
                    #[cfg(windows)]
                    raise_game_thread_priority();
                }
                game_loop(incoming, config, &thread_metrics, &loop_stop)
            })?;
        let handle = Self {
            to_game,
            next_id: Arc::new(AtomicI32::new(1)),
            metrics,
            stop: Arc::clone(&stop),
            default_game_mode,
            playerdata,
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

    /// Game mode every joining player gets (M4.5 Шаг 7.3).
    pub fn default_game_mode(&self) -> i32 {
        self.default_game_mode
    }

    /// The player save runtime of this server, if persistence is configured
    /// (Шаг 18a): the connection reads a player file from it at join.
    pub fn playerdata(&self) -> Option<&Arc<PlayerdataRuntime>> {
        self.playerdata.as_ref()
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
    let mut scheduler = Scheduler::with_grid(
        vec![Region::new(SINGLE_REGION)],
        RegionGrid::new(config.region_side),
        config.workers,
    );
    scheduler.set_store(config.store.clone());
    let mut recorder = Recorder::new(config.period);
    let mut replay = Replay::new(open_replay());
    // Autosave period in ticks; `0` disables the periodic save (Шаг 18a). The
    // quit of a single player and the shutdown save still run.
    let autosave_every = config
        .playerdata
        .as_ref()
        .map_or(0, |runtime| runtime.autosave_interval_ticks);
    // World autosave period (Шаг 22b): independent of the player save, `0`
    // disables the periodic pass; the shutdown save still runs.
    let world_autosave_every = config.world_autosave_ticks;
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
            &config,
        );
        let after_apply = Instant::now();
        let outcome = scheduler.tick(tick, deadline);
        // The mining timer advances after the regions (Шаг 16a.3), so a player
        // that crossed a region boundary is advanced in its new region.
        dig::tick(&mut scheduler);
        let after_sched = Instant::now();
        // Positions the tick applied (M4.2) are recorded after the queued
        // messages, so `seq` continues the per-tick counter that
        // `apply_queued` opened. Boundary crossings (M4.3 B) are applied after
        // the tick's positions, so they are recorded after them.
        record_moves(&mut replay, tick, &outcome.moves);
        record_transfers(&mut replay, tick, &outcome.transfers);
        let after_record = Instant::now();
        deliver_streams(&mut scheduler, &outcome.streams);
        let after_streams = Instant::now();
        deliver_entities(&mut scheduler, &outcome.entities);
        let after_entities = Instant::now();
        // Autosave (Шаг 18a): the snapshot is queued for the writer thread,
        // the tick never writes. The period is checked after the tick applied
        // every position, so the saved pose is the one this tick settled on.
        //
        // The world goes first (Шаг 22b, docs/WORLD-SAVE.md §6): the player
        // position is written after the world it stands in. The writer's
        // acknowledgements are drained here, before the new snapshot, so a
        // chunk the writer already stored is not queued twice.
        if world_autosave_every > 0 && tick.is_multiple_of(world_autosave_every) {
            if let (Some(save), Some(store)) = (config.worldsave.as_ref(), config.store.as_deref())
            {
                let started = Instant::now();
                for done in save.completed() {
                    store.saved(done);
                }
                let batch = store.autosave_batch(SAVE_CHUNKS_PER_PASS);
                let queued = batch.len();
                let retry = batch.clone();
                let accepted = queued == 0 || save.submit(batch);
                if !accepted {
                    store.not_queued(&retry);
                    warn!(
                        tick,
                        queued,
                        "world autosave queue is full; the chunks stay dirty for the next pass"
                    );
                }
                info!(
                    tick,
                    chunks = queued,
                    dirty = store.dirty_chunks(),
                    save_us = phase_us(started.elapsed()),
                    "world autosave queued"
                );
            }
        }
        if autosave_every > 0 && tick.is_multiple_of(autosave_every) {
            if let Some(runtime) = config.playerdata.as_deref() {
                // The tick only builds the snapshots and enqueues them; the
                // dedicated writer thread does the disk I/O. The duration is
                // logged once per autosave (Шаг 19a, live check 5) so the
                // scenario runner can show that enqueueing never approaches a
                // tick budget; `players` proves the snapshot covered the
                // connected players.
                let started = Instant::now();
                save_all(&scheduler, runtime);
                info!(
                    tick,
                    save_us = phase_us(started.elapsed()),
                    players = scheduler.players().count(),
                    "autosave queued"
                );
            }
        }
        for kick in &outcome.kicks {
            warn!(
                player = kick.id,
                reason = kick.reason.message(),
                "kicking a slow player"
            );
        }
        // Phase timing (M4.5 step 8): one warn line per tick that runs long.
        // `late_ms` is the tick's total lateness, measured from the deadline
        // to the moment the tick finished its work (`finished - deadline`),
        // so it grows with both a late wake and slow work. `wake_late_ms` is
        // only the wake-up delay (`woke - deadline`): the part the tick
        // cannot help. When `wake_late_ms` is large and `phases_total_us` is
        // small the stall is external (OS, console, antivirus); when the
        // phases are large the stall is this tick's own work and the phase
        // `*_us` fields name it. The line is written only past
        // `PHASE_LOG_MS`, so a healthy tick pays only a few `Instant` reads.
        let phases = TickPhases {
            apply_us: phase_us(after_apply.duration_since(work)),
            sched_us: phase_us(after_sched.duration_since(after_apply)),
            moves_us: phase_us(after_record.duration_since(after_sched)),
            streams_us: phase_us(after_streams.duration_since(after_record)),
            entities_us: phase_us(after_entities.duration_since(after_streams)),
        };
        let finished = after_entities;
        let late_ms = finished.saturating_duration_since(deadline);
        if late_ms > Duration::from_millis(PHASE_LOG_MS) {
            warn!(
                tick,
                late_ms = late_ms.as_secs_f64() * 1000.0,
                wake_late_ms = late.as_secs_f64() * 1000.0,
                phases_total_us = phases.total_us(),
                apply_us = phases.apply_us,
                sched_us = phases.sched_us,
                moves_us = phases.moves_us,
                streams_us = phases.streams_us,
                entities_us = phases.entities_us,
                n_moves = outcome.moves.len(),
                n_streams = outcome.streams.len(),
                n_entities = outcome.entities.len(),
                "tick phases"
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
    // Players still online at stop: capture their final state without queuing
    // disk work. The server first makes the final world durable, then queues
    // these staged snapshots and drains the player writer (Шаг 18a/22b).
    if let Some(runtime) = config.playerdata.as_deref() {
        stage_all(&scheduler, runtime);
    }
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
    config: &GameConfig,
) {
    replay.start_tick();
    while let Some(message) = next_message(incoming, disconnected) {
        apply(scheduler, replay, tick, message, config);
    }
}

/// Applies one incoming message to its owner region and records it.
///
/// The owner comes from the scheduler's `player_id → region_id` index, so a
/// `TeleportAck` or `Quit` after a `Transfer` reaches the new region while one
/// before it reaches the old one (ARCH-M4 §1, M4.3 B1).
fn apply<W: std::io::Write>(
    scheduler: &mut Scheduler,
    replay: &mut Replay<W>,
    tick: u64,
    message: ToGame,
    config: &GameConfig,
) {
    match message {
        ToGame::Join(join) => {
            let id = join.id;
            // Шаг 20a: one account, one session. The vanilla server drops the
            // older connection when the same profile logs in again: the S15
            // trace carries a play-state `disconnect` with
            // `multiplayer.disconnect.duplicate_login`, sent before the new
            // player appears. The offline UUID derives from the name, so the
            // comparison is the account, not the connection.
            let account = protocol::offline_uuid(&join.name);
            let stale: Vec<PlayerId> = scheduler
                .players()
                .filter(|player| player.uuid() == account)
                .map(|player| player.id())
                .collect();
            for old in stale {
                if old == id {
                    continue;
                }
                info!(
                    player = old,
                    name = %join.name,
                    "duplicate login: closing the older connection"
                );
                let _ = scheduler.deliver(
                    old,
                    ServerMessage::Disconnect {
                        reason: KickReason::DuplicateLogin,
                    },
                );
                // The kicked session still writes its state, like a quit; the
                // new connection has already read the file before this Join.
                if let Some(runtime) = config.playerdata.as_deref() {
                    save_player(scheduler, runtime, old);
                }
                if let Some(dig) = dig::take(scheduler, old) {
                    dig::reset(scheduler, old, dig.position);
                }
                scheduler.remove_player(old);
            }
            let payload = serde_json::json!({
                "name": &join.name,
                "view_distance": join.view_distance,
                "spawn": { "x": join.spawn.x, "y": join.spawn.y, "z": join.spawn.z },
            });
            // The region is the chunk of the spawn; the scheduler creates it
            // if this is the first player there (region_side > 0).
            let region_id = scheduler.insert_player(*join);
            replay.record(tick, Some(region_id), id, "join", payload);
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
            match scheduler.confirm_teleport(id) {
                Some(_) => info!(player = id, teleport_id, "teleport confirmed"),
                None => warn!(player = id, "teleport ack for an unknown player"),
            }
        }
        ToGame::Quit { id } => {
            let region_id = scheduler.region_of(id);
            replay.record(tick, region_id, id, "quit", serde_json::json!({}));
            // The player's final state goes to the save queue before the
            // region drops it (Шаг 18a).
            if let Some(runtime) = config.playerdata.as_deref() {
                save_player(scheduler, runtime, id);
            }
            // A dig left behind must stop showing cracks to its observers
            // (Шаг 16a.3); the state itself goes away with the player.
            if let Some(dig) = dig::take(scheduler, id) {
                dig::reset(scheduler, id, dig.position);
            }
            scheduler.remove_player(id);
        }
        ToGame::BlockAction(action) => {
            let rules = dig::DigRules {
                hardness: config.hardness.as_deref(),
                tolerance_ticks: config.dig_tolerance_ticks,
            };
            block_action::apply(
                scheduler,
                replay,
                tick,
                config.store.as_ref(),
                rules,
                action,
            );
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

/// Records the boundary crossings one tick applied to the replay journal.
///
/// A `transfer` line carries `{player, from, to}`; the tick number is the tick
/// the scheduler applied it in (ARCH-M4 §1, M4.3 B1). Shared by the live loop
/// and the replay tests so both write the same line.
fn record_transfers<W: std::io::Write>(replay: &mut Replay<W>, tick: u64, transfers: &[Transfer]) {
    for transfer in transfers {
        replay.record(
            tick,
            Some(transfer.from),
            transfer.player,
            "transfer",
            serde_json::json!({
                "player": transfer.player,
                "from": transfer.from,
                "to": transfer.to,
            }),
        );
    }
}

/// Delivers one tick's window moves to the connections (M4.4 B, ШАГ 2/3).
///
/// Runs **after** [`Scheduler::tick`], so the `player_id → region_id` index
/// already carries this tick's transfers and a player that crossed a boundary
/// gets its `stream` in the region that owns it from now on. The message is a
/// plain [`ServerMessage::StreamChunks`]; delivery uses the shared `try_send`
/// policy ([`Scheduler::deliver`]), so the tick never blocks and a full
/// outbound queue kicks the player. The journal is **not** extended —
/// streaming is transport state, not game state (M4.4 B).
fn deliver_streams(scheduler: &mut Scheduler, streams: &[StreamChunks]) {
    for stream in streams {
        let message = ServerMessage::StreamChunks {
            center: stream.center,
            add: stream.add.clone(),
            remove: stream.remove.clone(),
        };
        if let Some(kick) = scheduler.deliver(stream.player, message) {
            warn!(
                player = kick.id,
                reason = kick.reason.message(),
                "kicking a slow player"
            );
        }
    }
}

/// Delivers one tick's entity-visibility changes to the connections (M4.5
/// Шаг 3).
///
/// Runs **after** [`deliver_streams`], so the owner index is final. The
/// message is a plain [`ServerMessage::StreamEntities`]; delivery uses the
/// shared `try_send` policy ([`Scheduler::deliver`]). The journal is **not**
/// extended — entity streaming is transport state, not game state.
fn deliver_entities(scheduler: &mut Scheduler, events: &[StreamEntities]) {
    for event in events {
        let message = ServerMessage::StreamEntities {
            spawn: event.spawn.clone(),
            states: event.states.clone(),
            remove: event.remove.clone(),
            moves: event.moves.clone(),
            swings: event.swings.clone(),
        };
        if let Some(kick) = scheduler.deliver(event.player, message) {
            warn!(
                player = kick.id,
                reason = kick.reason.message(),
                "kicking a slow player"
            );
        }
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
