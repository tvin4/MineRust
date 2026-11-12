//! Shared play driver for the M4.5 two-bot vanilla trace (С1/С2/С3).
//!
//! One bot at a time runs its own loop here: join under its own name
//! (offline UUID derived from it), a full JSONL trace with the walk
//! format, and a tick-coupled 50 ms grid. The scenario logic lives in a
//! [`Behavior`] implemented by `tests/two_bot_trace.rs` — this module owns
//! transport and the core reactions only:
//!
//! * `minecraft:player_position` is acknowledged with the observed 33-byte
//!   layout; a second one is a rollback and ends the run;
//! * every `chunk_batch_finished` is answered with [`walk::PACE`] bytes;
//! * `client_tick_end` fires on the 50 ms absolute grid the first ack
//!   anchors, and `player_loaded` once at ack + 1001 ms — the live client's
//!   gap (`live-session.json` seq 52 → seq 91);
//! * movement comes from [`Behavior::on_tick`] right after `client_tick_end`
//!   (one movement packet per tick, never two in one tick);
//! * every JSONL line carries the socket wire time stamped by
//!   [`parity::Client`], never the line-write moment.
//!
//! Interfaces taken from the packet report (`C:\MineRust-data\reports\
//! packets.json`): `accept_teleportation` = 0, `client_tick_end` = 13,
//! `move_player_pos` = 30, `move_player_pos_rot` = 31, `move_player_rot` =
//! 32, `player_loaded` = 44.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32};
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use parity::walk::{self, Jsonl};
use parity::{
    from_hex, join_steps_named, Client, ParityEnv, TraceDirection, TraceEntry, PROBE_WAIT,
};
use protocol::{PacketKey, ProtocolState};

/// Player names of the two bots; offline UUIDs derive from them
/// (`protocol::offline_uuid`), and bot A identifies bot B's entity by the
/// same UUID in `minecraft:add_entity`.
pub const BOT_A: &str = "TraceBotA";
pub const BOT_B: &str = "TraceBotB";

/// One movement tick (20 per second, absolute grid — same as walk).
const TICK: Duration = Duration::from_millis(50);
/// First `player_loaded` after the first teleport ack: the live client's
/// gap of 1001.28 ms, floored (walk's [`walk`] constant re-derived here so
/// the two bots stay independent of walk's private timers).
const LOADED_DELAY: Duration = Duration::from_millis(1001);

/// Serverbound ids, resolved by name from the packets report (the
/// defaults below match the report's protocol ids).
const KEY_ACCEPT: PacketKey =
    PacketKey::serverbound(ProtocolState::Play, "minecraft:accept_teleportation", 0x00);
const KEY_PLAYER_LOADED: PacketKey =
    PacketKey::serverbound(ProtocolState::Play, "minecraft:player_loaded", 0x2C);
const KEY_TICK_END: PacketKey =
    PacketKey::serverbound(ProtocolState::Play, "minecraft:client_tick_end", 0x0D);
const KEY_MOVE_POS: PacketKey =
    PacketKey::serverbound(ProtocolState::Play, "minecraft:move_player_pos", 0x1E);
const KEY_MOVE_POS_ROT: PacketKey =
    PacketKey::serverbound(ProtocolState::Play, "minecraft:move_player_pos_rot", 0x1F);
const KEY_MOVE_ROT: PacketKey =
    PacketKey::serverbound(ProtocolState::Play, "minecraft:move_player_rot", 0x20);

/// Why a bot's loop ended — every value is a report fact.
#[derive(Debug, Clone)]
pub enum End {
    /// The scenario script finished on purpose.
    Script(String),
    /// The next movement would have left the ready zone (assert before
    /// send); in С3 this is the «tracking distance: не установлено» fact.
    Bounds(String),
    /// A second server `player_position`: the server rolled back.
    Rollback(usize),
    /// The server announced a disconnect with a reason (hex payload).
    Kick(String),
    /// The connection closed (transport error or plain drop).
    Closed(String),
    /// The run's safety deadline hit before the script finished.
    Timeout,
    /// Connect or the scripted join failed (fatal before play).
    Join(String),
    /// A play-state read/write or trace-file error.
    Io(String),
    /// The spawn packet did not have the observed layout.
    BadTeleport(usize),
}

impl End {
    /// One-line text for reports and summaries.
    pub fn text(&self) -> String {
        match self {
            End::Script(text) => format!("script done: {text}"),
            End::Bounds(text) => text.clone(),
            End::Rollback(count) => {
                format!("stop reason: rollback — server player_position #{count}")
            }
            End::Kick(payload) => format!("stop reason: kick: {payload}"),
            End::Closed(detail) => format!("stop reason: connection closed: {detail}"),
            End::Timeout => "stop reason: safety timeout".to_owned(),
            End::Join(detail) => format!("join failed: {detail}"),
            End::Io(detail) => format!("i/o: {detail}"),
            End::BadTeleport(size) => {
                format!("stop reason: player_position of {size} bytes did not decode")
            }
        }
    }

    /// Whether this end is the scripted, expected one.
    pub fn is_script(&self) -> bool {
        matches!(self, End::Script(_))
    }
}

/// Everything one bot produced, for the scenario report.
#[derive(Debug)]
pub struct BotReport {
    /// Why the loop ended.
    pub end: End,
    /// Server `player_position` packets seen (1 = no rollback).
    pub teleports: usize,
    /// Movement packets sent (all three flavours count once each).
    pub moves: u32,
    /// Monotonic milliseconds from the play loop start to the end.
    pub elapsed_ms: f64,
    /// The acknowledged spawn position.
    pub spawn: Option<[f64; 3]>,
    /// The last position the bot sent.
    pub last: Option<[f64; 3]>,
    /// `game_mode` decoded from the clientbound login (play) packet:
    /// `3` is spectator.
    pub game_mode: Option<i32>,
}

impl BotReport {
    /// A report that never entered play (used on early failures too).
    fn new(end: End) -> Self {
        Self {
            end,
            teleports: 0,
            moves: 0,
            elapsed_ms: 0.0,
            spawn: None,
            last: None,
            game_mode: None,
        }
    }
}

/// Facts the two bots share during one scenario: coordination flags plus
/// bot A's live entity-detection state (see `two_bot_trace.rs`).
///
/// Plain atomics — no `Mutex`: both loops poll them on their own 50 ms
/// grids and the values are single facts, not short read-modify-write
/// sequences across await points.
#[derive(Debug)]
pub struct Shared {
    /// Set when bot A has entered play (the driver starts B afterwards).
    pub a_joined: AtomicBool,
    /// Set when bot B has entered play (arms A's fallback entity match).
    pub b_joined: AtomicBool,
    /// Set by B while it walks forward — the only arm for A's
    /// lost-contact detection, so standing phases cannot trigger it.
    pub b_moving: AtomicBool,
    /// A saw `remove_entities` carrying B's entity id while B was walking.
    pub lost_contact: AtomicBool,
    /// B's scenario script is over (A counts its observation time from it).
    pub script_done: AtomicBool,
    /// B's entity id as A identified it; `-1` = not identified yet.
    pub b_entity: AtomicI32,
    /// `true` = the UUID hypothesis matched, `false` = the fallback
    /// "first add_entity after B joined" was used.
    pub uuid_strategy: AtomicBool,
    /// 0-based index of the `add_entity` that identified B (`-1` = none).
    pub entity_add_index: AtomicI32,
    /// `add_entity` packets seen by A.
    pub adds: AtomicU32,
    /// `remove_entities` packets seen by A whose id list parsed.
    pub removes: AtomicU32,
}

impl Shared {
    /// Fresh coordination state for one scenario.
    pub fn new() -> Self {
        Self {
            a_joined: AtomicBool::new(false),
            b_joined: AtomicBool::new(false),
            b_moving: AtomicBool::new(false),
            lost_contact: AtomicBool::new(false),
            script_done: AtomicBool::new(false),
            b_entity: AtomicI32::new(-1),
            uuid_strategy: AtomicBool::new(false),
            entity_add_index: AtomicI32::new(-1),
            adds: AtomicU32::new(0),
            removes: AtomicU32::new(0),
        }
    }
}

impl Default for Shared {
    fn default() -> Self {
        Self::new()
    }
}

/// What a scenario tells the bot to send outside the core reactions.
#[derive(Debug, Clone)]
pub enum Cmd {
    /// The scripted end (a clean, expected stop).
    Stop(String),
    /// Assert-before-send: the next step would leave the ready zone.
    Bounds(String),
    /// `move_player_pos`: absolute position (on-ground `true`, like walk's
    /// v3 walks — the bot flies as a spectator).
    MovePos([f64; 3]),
    /// `move_player_pos_rot`: position plus this tick's yaw/pitch.
    MovePosRot([f64; 3], f32, f32),
    /// `move_player_rot`: rotation only, no position change.
    MoveRot(f32, f32),
}

/// One bot's scenario logic; the loop here owns transport and timers.
pub trait Behavior {
    /// Called once the scripted join has entered play (coordination
    /// flags are the usual payload).
    fn on_joined(&mut self);

    /// The first acknowledged spawn position arrived.
    fn on_teleport(&mut self, pos: [f64; 3]);

    /// One received clientbound packet (everything else is core logic).
    fn on_packet(&mut self, entry: &TraceEntry, payload: &[u8]) -> Vec<Cmd>;

    /// After this bot's `client_tick_end` on the absolute grid: the
    /// movement decision. `pos` is `None` before the first teleport ack.
    fn on_tick(&mut self, loaded: bool, pos: Option<[f64; 3]>) -> Vec<Cmd>;
}

/// Mutable state of one bot's play loop (all fields are report facts).
#[derive(Debug, Default)]
struct Play {
    /// How many recorded packets were processed into lines and reactions.
    emitted: usize,
    /// Server `player_position` count (a second one is a rollback).
    teleports: usize,
    /// Movement packets sent.
    moves: u32,
    /// Current acknowledged/sent position.
    pos: Option<[f64; 3]>,
    /// First acknowledged position (the spawn).
    spawn: Option<[f64; 3]>,
    /// Last position sent.
    last: Option<[f64; 3]>,
    /// `game_mode` of the clientbound login (play).
    game_mode: Option<i32>,
    /// Next slot of the `client_tick_end` absolute 50 ms grid.
    tick_at: Option<tokio::time::Instant>,
    /// When `player_loaded` goes out (ack + [`LOADED_DELAY`]).
    loaded_send: Option<tokio::time::Instant>,
    /// Whether `player_loaded` has been sent (movement gate).
    loaded_sent: bool,
    /// Set by any stop; the loop breaks on the first `Some`.
    end: Option<End>,
}

/// Runs one bot: connect → scripted join under `name` → play loop →
/// JSONL close. Never fails the caller: connect/join/i/o problems come
/// back as [`End`] values in the report, like the loop's own stops.
pub async fn run_bot<B: Behavior + Send>(
    env: Arc<ParityEnv>,
    ids: Arc<PacketIds>,
    name: &'static str,
    trace_path: PathBuf,
    safety: Duration,
    mut behavior: B,
) -> BotReport {
    let port = match env.vanilla_port() {
        Ok(port) => port,
        Err(error) => return BotReport::new(End::Join(format!("vanilla port: {error}"))),
    };
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let mut client = match Client::connect(name, addr, Arc::clone(&ids)).await {
        Ok(client) => client,
        Err(error) => return BotReport::new(End::Join(format!("connect: {error}"))),
    };
    let steps = join_steps_named(&env, &ids, port, name, walk::VIEW_DISTANCE, PROBE_WAIT);
    if let Err(error) = client.run(&steps).await {
        return BotReport::new(End::Join(format!("join steps: {error}")));
    }
    behavior.on_joined();
    let mut jsonl = match Jsonl::create(&trace_path) {
        Ok(jsonl) => jsonl,
        Err(error) => return BotReport::new(End::Io(error.to_string())),
    };

    let started = std::time::Instant::now();
    let mut play = Play::default();
    loop {
        if started.elapsed() > safety {
            play.end = Some(End::Timeout);
            break;
        }
        // Wake on the nearest armed grid (tick, player_loaded); the read
        // itself may also return earlier whenever a packet arrives.
        let mut wake = tokio::time::Instant::now() + TICK;
        if let Some(at) = play.tick_at {
            wake = wake.min(at);
        }
        if let Some(at) = play.loaded_send {
            wake = wake.min(at);
        }
        if let Err(error) = client.recv_packet(wake).await {
            play.end = Some(End::Closed(error.to_string()));
            break;
        }
        if process(&mut client, &ids, &mut jsonl, &mut behavior, &mut play).await {
            break;
        }
        // `client_tick_end` on the absolute grid, then movement in the
        // same tick — exactly one movement packet per tick end.
        if let Some(at) = play.tick_at {
            if tokio::time::Instant::now() >= at {
                if let Err(error) = client.send_packet(ids.id(KEY_TICK_END), &[]).await {
                    play.end = Some(End::Io(error.to_string()));
                    break;
                }
                let mut next = at + TICK;
                while next <= tokio::time::Instant::now() {
                    next += TICK;
                }
                play.tick_at = Some(next);
                let cmds = behavior.on_tick(play.loaded_sent, play.pos);
                if apply(&mut client, &ids, &mut play, cmds).await {
                    break;
                }
            }
        }
        if let Some(at) = play.loaded_send {
            if tokio::time::Instant::now() >= at {
                let id = ids.id(KEY_PLAYER_LOADED);
                if let Err(error) = client.send_packet(id, &[]).await {
                    play.end = Some(End::Io(error.to_string()));
                    break;
                }
                play.loaded_send = None;
                play.loaded_sent = true;
            }
        }
    }

    // Anything still unrecorded must reach the trace and the facts before
    // the file closes. A rollback/kick/closed run keeps the end that cut
    // it off (walk behaves the same); a scripted or overlapped end drains.
    let drain = matches!(&play.end, None | Some(End::Script(_)));
    if drain {
        process(&mut client, &ids, &mut jsonl, &mut behavior, &mut play).await;
    }
    let mut report = BotReport::new(play.end.unwrap_or(End::Timeout));
    report.teleports = play.teleports;
    report.moves = play.moves;
    report.elapsed_ms = (started.elapsed().as_secs_f64() * 1_000_000.0).round() / 1000.0;
    report.spawn = play.spawn;
    report.last = play.last;
    report.game_mode = play.game_mode;
    if let Err(error) = jsonl.finish() {
        if report.end.is_script() {
            report.end = End::Io(error.to_string());
        } else {
            eprintln!("trace flush failed after {:?}: {error}", report.end);
        }
    }
    report
}

/// Processes every recorded-but-unprocessed packet: writes its JSONL line
/// and performs the core reactions. Returns `true` when the run must stop
/// (the end is stored in `play`).
async fn process<B: Behavior>(
    client: &mut Client,
    ids: &PacketIds,
    jsonl: &mut Jsonl,
    behavior: &mut B,
    play: &mut Play,
) -> bool {
    while play.emitted < client.trace().len() {
        // `stamps` and `trace.packets` grow together; a gap would fake
        // the whole timeline and is an error, never a guessed default.
        let Some(stamp) = client.stamps().get(play.emitted).copied() else {
            play.end = Some(End::Io(format!(
                "trace entry {} has no socket stamp",
                play.emitted
            )));
            return true;
        };
        let entry = client.trace().packets[play.emitted].clone();
        play.emitted += 1;
        if let Err(error) = jsonl.record(&entry, stamp) {
            play.end = Some(End::Io(error.to_string()));
            return true;
        }
        if entry.direction != TraceDirection::Clientbound {
            if entry
                .name
                .as_deref()
                .is_some_and(|name| name.starts_with("minecraft:move_player"))
            {
                play.moves += 1;
            }
            continue;
        }
        let payload = match from_hex(&entry.payload) {
            Ok(payload) => payload,
            Err(error) => {
                play.end = Some(End::Io(error.to_string()));
                return true;
            }
        };
        match entry.name.as_deref() {
            Some("minecraft:player_position") => {
                play.teleports += 1;
                if play.teleports > 1 {
                    play.end = Some(End::Rollback(play.teleports));
                    return true;
                }
                let Some((spawn, ack)) = walk::teleport_ack(&payload) else {
                    play.end = Some(End::BadTeleport(payload.len()));
                    return true;
                };
                play.pos = Some(spawn);
                play.spawn = Some(spawn);
                if let Err(error) = client.send_packet(ids.id(KEY_ACCEPT), &ack).await {
                    play.end = Some(End::Io(error.to_string()));
                    return true;
                }
                if play.tick_at.is_none() {
                    // The first ack anchors both timers (live trace order:
                    // ack → first tick → player_loaded at ack + 1001 ms).
                    let now = tokio::time::Instant::now();
                    play.tick_at = Some(now);
                    play.loaded_send = Some(now + LOADED_DELAY);
                }
                behavior.on_teleport(spawn);
            }
            Some("minecraft:chunk_batch_finished") => {
                let id = ids.id(protocol::ChunkBatchReceived::KEY);
                if let Err(error) = client.send_packet(id, &walk::PACE).await {
                    play.end = Some(End::Io(error.to_string()));
                    return true;
                }
            }
            Some("minecraft:disconnect") => {
                play.end = Some(End::Kick(entry.payload.clone()));
                return true;
            }
            Some("minecraft:login") => {
                play.game_mode = login_game_mode(&payload);
            }
            _ => {
                let cmds = behavior.on_packet(&entry, &payload);
                if apply(client, ids, play, cmds).await {
                    return true;
                }
            }
        }
    }
    false
}

/// Executes scenario commands; `true` = stop (stored in `play.end`).
async fn apply(client: &mut Client, ids: &PacketIds, play: &mut Play, cmds: Vec<Cmd>) -> bool {
    for cmd in cmds {
        let (key, payload, target) = match cmd {
            Cmd::Stop(text) => {
                play.end = Some(End::Script(text));
                return true;
            }
            Cmd::Bounds(text) => {
                play.end = Some(End::Bounds(text));
                return true;
            }
            Cmd::MovePos(pos) => (KEY_MOVE_POS, walk::move_payload(&pos, true), Some(pos)),
            Cmd::MovePosRot(pos, yaw, pitch) => (
                KEY_MOVE_POS_ROT,
                pos_rot_payload(&pos, yaw, pitch),
                Some(pos),
            ),
            Cmd::MoveRot(yaw, pitch) => (KEY_MOVE_ROT, rot_payload(yaw, pitch), None),
        };
        if let Err(error) = client.send_packet(ids.id(key), &payload).await {
            play.end = Some(End::Io(error.to_string()));
            return true;
        }
        if let Some(pos) = target {
            play.pos = Some(pos);
            play.last = Some(pos);
        }
        play.moves += 1;
    }
    false
}

/// `game_mode` of a clientbound login (play) payload; `None` when the
/// layout does not decode (the mode then cannot be proven).
fn login_game_mode(payload: &[u8]) -> Option<i32> {
    let mut reader = protocol::PacketReader::new(payload);
    protocol::LoginPlay::decode_payload(&mut reader)
        .ok()
        .map(|login| login.game_mode)
}

/// Payload of `move_player_pos_rot`: 3 big-endian doubles + yaw f32 +
/// pitch f32 + on-ground byte (33 bytes — same size and shape as the
/// live client's record in `live-session.json`, with this tick's angles).
fn pos_rot_payload(p: &[f64; 3], yaw: f32, pitch: f32) -> Vec<u8> {
    let mut out = Vec::with_capacity(33);
    for value in p {
        out.extend_from_slice(&value.to_be_bytes());
    }
    out.extend_from_slice(&yaw.to_be_bytes());
    out.extend_from_slice(&pitch.to_be_bytes());
    out.push(1);
    out
}

/// Payload of `move_player_rot`: yaw f32 + pitch f32 + on-ground byte
/// (9 bytes) — the live client's layout, observed as
/// `c12cccce40b6666801` (live-session.json id 32).
fn rot_payload(yaw: f32, pitch: f32) -> Vec<u8> {
    let mut out = Vec::with_capacity(9);
    out.extend_from_slice(&yaw.to_be_bytes());
    out.extend_from_slice(&pitch.to_be_bytes());
    out.push(1);
    out
}

/// Reads an `add_entity`/`remove_entities` id list the way the wire
/// carries it: VarInt count, then that many VarInt ids. For `add_entity`
/// the caller wants the leading entity id of the documented layout.
pub fn decode_ids(payload: &[u8]) -> Result<Vec<i32>, String> {
    let (count, mut at) = protocol::decode_varint(payload).map_err(|error| error.to_string())?;
    if !(0..=4096).contains(&count) {
        return Err(format!("implausible id count {count}"));
    }
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let (id, used) = protocol::decode_varint(payload.get(at..).ok_or("id list truncated")?)
            .map_err(|error| error.to_string())?;
        out.push(id);
        at += used;
    }
    Ok(out)
}

/// Whether a block position lies in the v3 ready zone: the central chunk
/// rectangle (x −14..26, z −22..18) of the frozen 65×65 template, the same
/// numbers walk's `in_zone_v3` checks (window radius 10 stays prepared).
pub fn in_ready_zone(x: f64, z: f64) -> bool {
    let cx = (x.floor() as i64).div_euclid(16) as i32;
    let cz = (z.floor() as i64).div_euclid(16) as i32;
    (-14..=26).contains(&cx) && (-22..=18).contains(&cz)
}
