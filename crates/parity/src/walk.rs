//! The walk-trace bot for M4.0 (a): a scripted client that joins vanilla,
//! keeps the connection alive, walks only inside the ready zone of the
//! template and records every packet with a monotonic timestamp.
//!
//! Behaviors in play (all observable formats come from the live client's
//! trace `target/parity/live-session.json`, never from the wiki):
//!
//! * keep-alives are acknowledged by [`crate::Client`] on the way in;
//! * every `chunk_batch_finished` is answered with `chunk_batch_received`
//!   carrying the pace float of the live trace (byte exact);
//! * `minecraft:player_position` is acknowledged with the 33-byte layout
//!   observed there (VarInt teleport id + 3 doubles + 8 zero bytes);
//! * `minecraft:client_tick_end` (serverbound id 13 in the packets
//!   report) goes out every 50 ms on the absolute grid the first
//!   teleport ack anchors, and `minecraft:player_loaded` once at
//!   ack + 1001 ms — the order and gap of the live client
//!   (`live-session.json`: ack seq 52 → first tick seq 53 → player_loaded
//!   seq 91 = +1001.28 ms);
//! * `minecraft:move_player_pos` goes out once per 50 ms absolute deadline,
//!   step ≤ 0.2 blocks, constant height, and any step that would leave the
//!   owner's zone is refused *before* it is sent;
//! * every JSONL line carries the packet's monotonic wire time — elapsed
//!   since the connection attempt, stamped by [`Client`] at the socket
//!   read/write, never at the moment the line happens to be written;
//! * every server `player_position` after the first one is a rollback and
//!   ends the run immediately, as does a disconnect, a closed connection
//!   or the safety deadline.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use protocol::{LoginPlay, PacketKey, ProtocolState};

use crate::client::{Client, ClientError};
use crate::env::{EnvError, ParityEnv};
use crate::steps::join_steps_with_view;
use crate::trace::{from_hex, TraceDirection, TraceEntry, TraceState};

/// View distance the bot asks for (the copy's `view-distance=10`).
pub const VIEW_DISTANCE: i8 = 10;

/// Pace float of `chunk_batch_received`, byte-exact from the live client
/// trace (`live-session.json` seq 81, hex `40957d29`, ≈ 4.6715).
/// **Depends on the bot**: it is what that client happened to send.
const PACE: [u8; 4] = [0x40, 0x95, 0x7D, 0x29];

/// One movement tick of the bot (20 per second, absolute deadline grid).
const TICK: Duration = Duration::from_millis(50);

/// Straight step along +x: 0.2 blocks, the owner's per-tick cap.
const STRAIGHT: f64 = 0.2;
/// Diagonal step per axis; the length is 0.2 blocks.
const DIAG: f64 = 0.2 / std::f64::consts::SQRT_2;

/// Allowed central chunk, x axis (owner's constant).
const CHUNK_X: (i32, i32) = (3, 9);
/// Allowed central chunk, z axis.
const CHUNK_Z: (i32, i32) = (-5, 1);
/// Allowed block positions, x: block edges 48..=159 minus a 2-block reserve.
const BLOCK_X: (f64, f64) = (50.0, 157.0);
/// Allowed block positions, z: block edges -80..=31 minus a 2-block reserve.
const BLOCK_Z: (f64, f64) = (-78.0, 29.0);

/// Serverbound ids, resolved by name from the packets report (default ids
/// matter only when no report is configured).
const KEY_ACCEPT: PacketKey =
    PacketKey::serverbound(ProtocolState::Play, "minecraft:accept_teleportation", 0x00);
const KEY_PLAYER_LOADED: PacketKey =
    PacketKey::serverbound(ProtocolState::Play, "minecraft:player_loaded", 0x2C);
/// Exactly 13 in the packets report (walk default matches it; the report
/// wins whenever one is configured).
const KEY_TICK_END: PacketKey =
    PacketKey::serverbound(ProtocolState::Play, "minecraft:client_tick_end", 0x0D);

/// Delay between the first teleport ack and `player_loaded`: the live
/// client's gap of 1001.28 ms (`live-session.json` seq 52 → seq 91),
/// floored to a whole millisecond.
const LOADED_DELAY: Duration = Duration::from_millis(1001);

/// Game mode id that means spectator in the login (play) packet.
/// SOURCE: protocol docs (wiki, unverified by us) — cross-checked in the
/// trace by [`SPECTATOR_FLAGS`], the two facts must agree (ЧАСТЬ I, п. 5).
const SPECTATOR_GAME_MODE: i32 = 3;

/// The `player_abilities` flags byte recorded in run P1 under the
/// forced-spectator configuration (server log: `Default game type:
/// SPECTATOR`): invulnerable `0x01` + flying `0x02` + may-fly `0x04`,
/// no instant build. The creative/wrong-mode bits disagree with login.
const SPECTATOR_FLAGS: u8 = 0x07;
const KEY_MOVE_POS: PacketKey =
    PacketKey::serverbound(ProtocolState::Play, "minecraft:move_player_pos", 0x1E);

/// What a run was asked to do.
#[derive(Debug, Clone, Copy)]
pub enum WalkMode {
    /// Stands still for `hold`, then leaves.
    Stand {
        /// How long the bot idles in play before quitting.
        hold: Duration,
    },
    /// `steps` straight moves along +x, then stands for `settle`.
    Straight {
        /// Number of movement packets to send.
        steps: u32,
        /// Idle time after the last step.
        settle: Duration,
    },
    /// `steps` diagonal moves (+x, -z), then stands for `settle`.
    Diagonal {
        /// Number of movement packets to send.
        steps: u32,
        /// Idle time after the last step.
        settle: Duration,
    },
}

/// Why the play loop ended — every value here is a report fact, never a
/// tuned parameter.
#[derive(Debug, Clone, PartialEq)]
pub enum Stop {
    /// The mode ran to its planned end.
    Completed,
    /// A second server `player_position`: the server rolled back.
    Rollback {
        /// How many server `player_position` packets were seen in total.
        count: usize,
    },
    /// The next step would have left the zone (assert before send).
    Bounds {
        /// Refused x.
        x: f64,
        /// Refused z.
        z: f64,
    },
    /// The server closed the connection (kick or plain drop).
    Closed {
        /// Transport error text.
        detail: String,
    },
    /// The server announced a disconnect with a reason.
    Kick {
        /// Raw reason payload, hex.
        payload: String,
    },
    /// No `player_position` in time, or the run's safety deadline hit.
    TimedOut {
        /// Server `player_position` count at the moment of the timeout.
        teleports: usize,
    },
    /// The spawn packet did not have the trace's layout: stop, report.
    BadTeleport {
        /// Payload size seen.
        size: usize,
    },
    /// The trace's mode facts failed the spectator cross-check of
    /// ЧАСТЬ I, п. 5 (`login.game_mode` vs `player_abilities` flags).
    Mode {
        /// The offending fact, human-readable (report line).
        detail: String,
    },
}

/// One finished chunk batch (pace stream).
#[derive(Debug, Clone)]
pub struct Batch {
    /// Monotonic milliseconds since the connection attempt: the wire time
    /// of the closing `chunk_batch_finished` (the trace's `t_ms`).
    pub t_ms: f64,
    /// `level_chunk_with_light` packets since the previous `start`.
    pub chunks: u64,
    /// Their payload bytes.
    pub bytes: u64,
}

/// One window-move of the server's loaded-chunk view, with the bot's
/// monotonic time (tick number = t_ms / 50 in the report; the clock is
/// the connection one, so tick 0 is the connection, not the join).
///
/// Coordinates come from decoding the recorded payload as two VarInts
/// `(x, z)` — the layout observed in the live trace (`06feffffff0f` = the
/// spawn chunk 6, −2). A payload that does not decode keeps `x = z =
/// [`UNKNOWN_CHUNK`] and the report prints it as unparsed.
#[derive(Debug, Clone)]
pub struct CacheMove {
    /// Monotonic milliseconds since the connection attempt: the wire time
    /// of the packet (the trace's `t_ms`).
    pub t_ms: f64,
    /// Packet name (`set_chunk_cache_center` / `forget_level_chunk`).
    pub name: String,
    /// Chunk x, or [`UNKNOWN_CHUNK`].
    pub x: i32,
    /// Chunk z, or [`UNKNOWN_CHUNK`].
    pub z: i32,
}

/// Sentinel `x`/`z` of a cache-move payload that did not decode.
pub const UNKNOWN_CHUNK: i32 = i32::MIN;

/// Play-loop timers armed together by the first teleport ack
/// (ЧАСТЬ I, п. 2). Before that ack neither packet may go out.
#[derive(Debug, Default)]
struct Timers {
    /// Next slot of the `client_tick_end` 50 ms absolute grid: `None`
    /// until the ack anchors it at the ack instant.
    tick: Option<tokio::time::Instant>,
    /// When `player_loaded` goes out (`None` = not armed or already
    /// sent): the ack instant + [`LOADED_DELAY`].
    loaded: Option<tokio::time::Instant>,
}

/// Everything the run produced, for the report.
#[derive(Debug)]
pub struct RunStats {
    /// Packet counts keyed `"<direction>/<name or id>"`.
    pub counts: BTreeMap<String, u64>,
    /// Server `player_position` packets seen (must be exactly 1).
    pub teleports: usize,
    /// Movement packets sent.
    pub moves: u32,
    /// Server window moves, in arrival order.
    pub cache_moves: Vec<CacheMove>,
    /// Finished chunk batches with their sizes.
    pub batches: Vec<Batch>,
    /// Why the loop ended.
    pub stop: Stop,
    /// Monotonic milliseconds from the play loop start to the quit.
    pub elapsed_ms: f64,
    /// The acknowledged spawn position, when one arrived.
    pub spawn: Option<[f64; 3]>,
    /// The last position the bot sent.
    pub last: Option<[f64; 3]>,
    /// `game_mode` decoded from the clientbound login (play) packet:
    /// `3` is spectator (ЧАСТЬ I, п. 5).
    pub game_mode: Option<i32>,
    /// Flags byte of the clientbound `player_abilities` packet (`0x07`
    /// under the forced spectator of the observed runs).
    pub abilities_flags: Option<u8>,
}

/// Failures outside the run's stop conditions (connect, join, file).
#[derive(Debug, thiserror::Error)]
pub enum WalkError {
    /// The parity environment could not be read (port, paths).
    #[error("parity environment: {0}")]
    Env(#[from] EnvError),
    /// The TCP connection could not be established.
    #[error("connect: {0}")]
    Connect(#[source] ClientError),
    /// The join script failed (login, configuration, first packets).
    #[error("join script: {0}")]
    Join(#[source] ClientError),
    /// A play-state read/write failed.
    #[error("play i/o: {0}")]
    Io(#[source] ClientError),
    /// The JSONL trace file could not be created or written.
    #[error("trace {path}: {source}")]
    Jsonl {
        /// Trace path involved.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A recorded packet could not be turned back into bytes.
    #[error("recorded payload is not hex: {0}")]
    Hex(#[from] crate::trace::HexError),
    /// A trace entry has no socket timestamp: packets and stamps are
    /// pushed together by [`Client::record`], so a gap would fake the
    /// whole timeline of the run.
    #[error("trace entry {0} has no socket stamp")]
    Stamp(usize),
}

/// The JSONL trace writer: one line per packet, monotonic wire time.
struct Jsonl {
    file: std::io::BufWriter<std::fs::File>,
    seq: u64,
}

impl Jsonl {
    /// Creates (and truncates) the trace file.
    fn create(path: &Path) -> Result<Self, WalkError> {
        let display = path.display().to_string();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| WalkError::Jsonl {
                path: display.clone(),
                source,
            })?;
        }
        let file = std::fs::File::create(path).map_err(|source| WalkError::Jsonl {
            path: display,
            source,
        })?;
        Ok(Self {
            file: std::io::BufWriter::new(file),
            seq: 0,
        })
    }

    /// Appends one packet line: time, direction, state, id, name, base64.
    ///
    /// `stamp` is the packet's socket time (elapsed since the connection
    /// attempt), taken by [`Client`] when the packet crossed the wire —
    /// the line's own write moment never enters the trace.
    fn record(&mut self, entry: &TraceEntry, stamp: Duration) -> Result<(), WalkError> {
        self.seq += 1;
        let line = serde_json::json!({
            "seq": self.seq,
            "t_ms": ms(stamp),
            "dir": direction_name(entry.direction),
            "state": state_name(entry.state),
            "id": entry.id,
            "name": entry.name,
            "b64": base64(&from_hex(&entry.payload)?),
        });
        serde_json::to_writer(&mut self.file, &line).map_err(|source| WalkError::Jsonl {
            path: "trace".to_owned(),
            source: std::io::Error::other(source),
        })?;
        writeln!(self.file, "\n").map_err(|source| WalkError::Jsonl {
            path: "trace".to_owned(),
            source,
        })?;
        Ok(())
    }

    /// Flushes buffered lines before the handle is closed.
    fn finish(mut self) -> Result<(), WalkError> {
        self.file.flush().map_err(|source| WalkError::Jsonl {
            path: "trace".to_owned(),
            source,
        })
    }
}

/// Runs one walk: connects, joins with view distance 10, plays the mode.
pub async fn run_walk(
    env: &ParityEnv,
    ids: Arc<PacketIds>,
    mode: WalkMode,
    trace_path: &Path,
) -> Result<RunStats, WalkError> {
    let port = env.vanilla_port()?;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut client = Client::connect("vanilla-walk", addr, Arc::clone(&ids))
        .await
        .map_err(WalkError::Connect)?;
    let steps = join_steps_with_view(env, &ids, port, VIEW_DISTANCE);
    client.run(&steps).await.map_err(WalkError::Join)?;

    let mut jsonl = Jsonl::create(trace_path)?;
    let stats = play(&mut client, &ids, mode, &mut jsonl).await;
    jsonl.finish()?;
    stats
}

/// The play loop: read, react, keep the 50 ms send grid, quit on stop.
async fn play(
    client: &mut Client,
    ids: &PacketIds,
    mode: WalkMode,
    jsonl: &mut Jsonl,
) -> Result<RunStats, WalkError> {
    let (hold, steps, settle) = match mode {
        WalkMode::Stand { hold } => (hold, 0, Duration::ZERO),
        WalkMode::Straight { steps, settle } => (Duration::ZERO, steps, settle),
        WalkMode::Diagonal { steps, settle } => (Duration::ZERO, steps, settle),
    };
    let started = std::time::Instant::now();
    let safety = TICK * steps + hold + settle + Duration::from_secs(60);
    // Set on every `break` (the loop's only exit), so no default is needed.
    let mut stop: Option<Stop>;
    let mut stats = new_stats();
    let mut emitted = 0usize;
    let mut pos: Option<[f64; 3]> = None;
    let mut spawn: Option<[f64; 3]> = None;
    let mut timers = Timers::default();
    let mut moved = 0u32;
    let mut chunking = Batch {
        t_ms: 0.0,
        chunks: 0,
        bytes: 0,
    };
    let mut settle_at = if steps == 0 {
        Some(tokio::time::Instant::now() + hold)
    } else {
        None
    };
    let mut deadline = tokio::time::Instant::now() + TICK;

    loop {
        if started.elapsed() > safety {
            stop = Some(Stop::TimedOut {
                teleports: stats.teleports,
            });
            break;
        }
        if let Some(at) = settle_at {
            if tokio::time::Instant::now() >= at {
                stop = Some(Stop::Completed);
                break;
            }
        }
        // Wake on the nearest of the three grids (movement, tick_end,
        // player_loaded): the other timers are armed later than this
        // read, and a read parked until the movement grid would delay
        // an already-due `client_tick_end` by up to a full tick.
        let mut wake = deadline;
        if let Some(at) = timers.tick {
            wake = wake.min(at);
        }
        if let Some(at) = timers.loaded {
            wake = wake.min(at);
        }
        match client.recv_packet(wake).await {
            Ok(_) => {}
            Err(error) => {
                stop = Some(Stop::Closed {
                    detail: error.to_string(),
                });
                break;
            }
        }
        let processed = process_recorded(
            client,
            ids,
            jsonl,
            &mut stats,
            &mut emitted,
            &mut pos,
            &mut timers,
            &mut chunking,
        )
        .await?;
        if let Some(reason) = processed {
            stop = Some(reason);
            break;
        }
        if spawn.is_none() {
            spawn = pos;
        }
        // ЧАСТЬ I, п. 2: `client_tick_end` (id 13) on the absolute
        // 50 ms grid the first ack anchored, then `player_loaded` once
        // at ack + 1001 ms — the order of the live trace (seq 53…90,
        // then seq 91).
        if let Some(at) = timers.tick {
            if tokio::time::Instant::now() >= at {
                let id = ids.id(KEY_TICK_END);
                send(client, id, &[]).await?;
                let mut next = at + TICK;
                while next <= tokio::time::Instant::now() {
                    next += TICK;
                }
                timers.tick = Some(next);
            }
        }
        if let Some(at) = timers.loaded {
            if tokio::time::Instant::now() >= at {
                let id = ids.id(KEY_PLAYER_LOADED);
                send(client, id, &[]).await?;
                timers.loaded = None;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            let next = next_step(mode, pos, moved);
            match next {
                Some(target) if in_zone(target[0], target[2]) => {
                    let id = ids.id(KEY_MOVE_POS);
                    send(client, id, &move_payload(&target)).await?;
                    pos = Some(target);
                    moved += 1;
                    if moved == steps {
                        settle_at = Some(tokio::time::Instant::now() + settle);
                    }
                }
                Some(target) => {
                    stop = Some(Stop::Bounds {
                        x: target[0],
                        z: target[2],
                    });
                    break;
                }
                None => {}
            }
            // Absolute grid: a missed slot is skipped, never bursted.
            let mut next_deadline = deadline + TICK;
            if next_deadline <= tokio::time::Instant::now() {
                next_deadline = tokio::time::Instant::now() + TICK;
            }
            deadline = next_deadline;
        }
    }

    // Anything still unrecorded (the final move, late packet) must be in
    // the trace and the counters; a rollback seen on the way out still
    // overrides a plain `Completed`.
    if stop.is_none() || stop == Some(Stop::Completed) {
        let last = process_recorded(
            client,
            ids,
            jsonl,
            &mut stats,
            &mut emitted,
            &mut pos,
            &mut timers,
            &mut chunking,
        )
        .await?;
        if let Some(reason) = last {
            stop = Some(reason);
        }
    }
    stats.stop = stop.unwrap_or(Stop::Completed);
    stats.elapsed_ms = ms(started.elapsed());
    stats.spawn = spawn;
    if stats.moves > 0 {
        stats.last = pos;
    }
    Ok(stats)
}

/// Handles every recorded-but-not-yet-processed packet, sends reactions.
#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
async fn process_recorded(
    client: &mut Client,
    ids: &PacketIds,
    jsonl: &mut Jsonl,
    stats: &mut RunStats,
    emitted: &mut usize,
    pos: &mut Option<[f64; 3]>,
    timers: &mut Timers,
    chunking: &mut Batch,
) -> Result<Option<Stop>, WalkError> {
    while *emitted < client.trace().len() {
        // The wire time of this packet: `stamps` and `trace.packets`
        // grow together, so the indexes match — a gap is an invariant
        // break, never something to paper over with a default.
        let Some(stamp) = client.stamps().get(*emitted).copied() else {
            return Err(WalkError::Stamp(*emitted));
        };
        let entry = client.trace().packets[*emitted].clone();
        *emitted += 1;
        let key = count_key(&entry);
        *stats.counts.entry(key).or_insert(0) += 1;
        jsonl.record(&entry, stamp)?;

        if entry.direction != TraceDirection::Clientbound {
            if entry.name.as_deref() == Some("minecraft:move_player_pos") {
                stats.moves += 1;
            }
            continue;
        }
        let payload = from_hex(&entry.payload)?;
        match entry.name.as_deref() {
            Some("minecraft:player_position") => {
                stats.teleports += 1;
                if stats.teleports > 1 {
                    return Ok(Some(Stop::Rollback {
                        count: stats.teleports,
                    }));
                }
                let Some((spawn_pos, ack)) = teleport_ack(&payload) else {
                    return Ok(Some(Stop::BadTeleport {
                        size: payload.len(),
                    }));
                };
                *pos = Some(spawn_pos);
                let id = ids.id(KEY_ACCEPT);
                send(client, id, &ack).await?;
                if timers.tick.is_none() {
                    // ЧАСТЬ I, п. 2: the first ack anchors both timers,
                    // like the live trace (ack seq 52, first tick seq 53,
                    // player_loaded seq 91 = ack + 1001.28 ms).
                    let now = tokio::time::Instant::now();
                    timers.tick = Some(now);
                    timers.loaded = Some(now + LOADED_DELAY);
                }
            }
            Some("minecraft:chunk_batch_finished") => {
                let id = ids.id(protocol::ChunkBatchReceived::KEY);
                send(client, id, &PACE).await?;
                let mut done = std::mem::replace(
                    chunking,
                    Batch {
                        t_ms: 0.0,
                        chunks: 0,
                        bytes: 0,
                    },
                );
                done.t_ms = ms(stamp);
                stats.batches.push(done);
            }
            Some("minecraft:level_chunk_with_light") => {
                chunking.chunks += 1;
                chunking.bytes += payload.len() as u64;
            }
            Some(name @ ("minecraft:set_chunk_cache_center" | "minecraft:forget_level_chunk")) => {
                let (x, z) = chunk_center(&payload);
                stats.cache_moves.push(CacheMove {
                    t_ms: ms(stamp),
                    name: name.to_owned(),
                    x,
                    z,
                });
            }
            Some("minecraft:login") => {
                // ЧАСТЬ I, п. 5: the mode is proven by the trace itself —
                // the login (play) field, never by console probes.
                let Some(mode) = login_game_mode(&payload) else {
                    return Ok(Some(Stop::Mode {
                        detail: format!("login payload ({} bytes) did not decode", payload.len()),
                    }));
                };
                stats.game_mode = Some(mode);
                if mode != SPECTATOR_GAME_MODE {
                    return Ok(Some(Stop::Mode {
                        detail: format!(
                            "login game_mode {mode}; expected {SPECTATOR_GAME_MODE} (spectator)"
                        ),
                    }));
                }
            }
            Some("minecraft:player_abilities") => {
                let Some(flags) = payload.first().copied() else {
                    return Ok(Some(Stop::Mode {
                        detail: "player_abilities payload is empty".to_owned(),
                    }));
                };
                stats.abilities_flags = Some(flags);
                if flags != SPECTATOR_FLAGS {
                    return Ok(Some(Stop::Mode {
                        detail: format!(
                            "player_abilities flags 0x{flags:02X}; expected \
                             0x{SPECTATOR_FLAGS:02X} (spectator)"
                        ),
                    }));
                }
            }
            Some("minecraft:disconnect") => {
                return Ok(Some(Stop::Kick {
                    payload: entry.payload.clone(),
                }));
            }
            _ => {}
        }
    }
    Ok(None)
}

/// `game_mode` of a clientbound login (play) payload; `None` when the
/// layout does not decode (the mode then cannot be proven).
fn login_game_mode(payload: &[u8]) -> Option<i32> {
    let mut reader = protocol::PacketReader::new(payload);
    LoginPlay::decode_payload(&mut reader)
        .ok()
        .map(|login| login.game_mode)
}

/// Sends one packet; the trace entry it creates is picked up by the
/// caller's next pass over the recorded packets.
async fn send(client: &mut Client, id: u32, payload: &[u8]) -> Result<(), WalkError> {
    client.send_packet(id, payload).await.map_err(WalkError::Io)
}

/// Fresh counters of one run.
fn new_stats() -> RunStats {
    RunStats {
        counts: BTreeMap::new(),
        teleports: 0,
        moves: 0,
        cache_moves: Vec::new(),
        batches: Vec::new(),
        stop: Stop::Completed,
        elapsed_ms: 0.0,
        spawn: None,
        last: None,
        game_mode: None,
        abilities_flags: None,
    }
}

/// Monotonic milliseconds of a socket stamp, rounded to microseconds
/// (the wire clock of the JSONL trace, of the report tables and of
/// [`RunStats::elapsed_ms`]).
///
/// The rounding happens in microseconds and only then converts to
/// milliseconds: rounding milliseconds and dividing by 1000 again would
/// return seconds (a 10.8 s run read as `t_ms = 10.803`).
fn ms(stamp: Duration) -> f64 {
    (stamp.as_secs_f64() * 1_000_000.0).round() / 1000.0
}

/// Counter key of one recorded packet: direction plus name (or raw id).
fn count_key(entry: &TraceEntry) -> String {
    let name = match entry.name.as_deref() {
        Some(name) => name.to_owned(),
        None => format!("id:{}", entry.id),
    };
    format!("{} / {name}", direction_name(entry.direction))
}

/// Wire name of a direction for the JSONL lines.
fn direction_name(direction: TraceDirection) -> &'static str {
    match direction {
        TraceDirection::Clientbound => "clientbound",
        TraceDirection::Serverbound => "serverbound",
    }
}

/// Wire name of a protocol state for the JSONL lines.
fn state_name(state: TraceState) -> &'static str {
    match state {
        TraceState::Handshake => "handshake",
        TraceState::Status => "status",
        TraceState::Login => "login",
        TraceState::Configuration => "configuration",
        TraceState::Play => "play",
    }
}

/// Standard base64 with padding (JSONL payloads).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).map_or(0, |&v| u32::from(v));
        let b2 = chunk.get(2).map_or(0, |&v| u32::from(v));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(triple >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(triple >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(triple >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(triple & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// The next planned position: `None` when the mode does not walk.
fn next_step(mode: WalkMode, pos: Option<[f64; 3]>, moved: u32) -> Option<[f64; 3]> {
    let current = pos?;
    match mode {
        WalkMode::Stand { .. } => None,
        WalkMode::Straight { steps, .. } if moved < steps => {
            Some([current[0] + STRAIGHT, current[1], current[2]])
        }
        WalkMode::Diagonal { steps, .. } if moved < steps => {
            Some([current[0] + DIAG, current[1], current[2] - DIAG])
        }
        _ => None,
    }
}

/// Whether a block position satisfies the owner's zone: central chunk
/// within the allowed range and a 2-block reserve from the edges.
fn in_zone(x: f64, z: f64) -> bool {
    let cx = (x.floor() as i64).div_euclid(16) as i32;
    let cz = (z.floor() as i64).div_euclid(16) as i32;
    (BLOCK_X.0..=BLOCK_X.1).contains(&x)
        && (BLOCK_Z.0..=BLOCK_Z.1).contains(&z)
        && (CHUNK_X.0..=CHUNK_X.1).contains(&cx)
        && (CHUNK_Z.0..=CHUNK_Z.1).contains(&cz)
}

/// Payload of `minecraft:move_player_pos`: 3 big-endian doubles plus the
/// on-ground byte (size 25, as in the live client's trace; `false` — the
/// bot flies as a spectator).
fn move_payload(p: &[f64; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity(25);
    for value in p {
        out.extend_from_slice(&value.to_be_bytes());
    }
    out.push(0);
    out
}

/// Decodes a window-move payload as two VarInts `(x, z)`; anything else
/// is reported as [`UNKNOWN_CHUNK`] instead of guessed at.
fn chunk_center(payload: &[u8]) -> (i32, i32) {
    let Ok((first, used)) = protocol::decode_varint(payload) else {
        return (UNKNOWN_CHUNK, UNKNOWN_CHUNK);
    };
    let Ok((second, _)) = protocol::decode_varint(&payload[used..]) else {
        return (UNKNOWN_CHUNK, UNKNOWN_CHUNK);
    };
    (first, second)
}

/// Parses a server `player_position` and builds the acknowledgment the
/// live client sent: the echoed VarInt id + 3 doubles + 8 zero bytes.
fn teleport_ack(payload: &[u8]) -> Option<([f64; 3], Vec<u8>)> {
    let (_, consumed) = protocol::decode_varint(payload).ok()?;
    let end = consumed.checked_add(24)?;
    if payload.len() < end {
        return None;
    }
    let read = |offset: usize| -> Option<f64> {
        let slice: [u8; 8] = payload[consumed + offset..consumed + offset + 8]
            .try_into()
            .ok()?;
        Some(f64::from_be_bytes(slice))
    };
    let spawn = [read(0)?, read(8)?, read(16)?];
    let mut ack = payload[..end].to_vec();
    ack.extend_from_slice(&[0; 8]);
    Some((spawn, ack))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ms_is_milliseconds_with_microsecond_precision() {
        assert_eq!(ms(Duration::ZERO), 0.0);
        assert_eq!(ms(Duration::from_millis(50)), 50.0);
        assert_eq!(ms(Duration::from_micros(1_500)), 1.5);
        // The bug this guards: a 10.803 s stamp used to read as 10.803 ms.
        assert_eq!(
            ms(Duration::from_secs(10) + Duration::from_millis(803)),
            10_803.0
        );
    }

    #[test]
    fn the_run_lifetime_is_milliseconds_too() {
        // `elapsed_ms` goes through the same helper: the 10 s P1 hold must
        // read as ~10 000 ms, never as ~10.
        assert_eq!(ms(Duration::from_secs_f64(10.8)), 10_800.0);
    }
}
