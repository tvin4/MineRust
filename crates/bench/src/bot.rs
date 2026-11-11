//! The bench bot (ARCH-M4 §5, A2.1): a light client that joins through the
//! real handshake → login → configuration → play sequence and then keeps
//! the connection alive while sending moves on the tick grid.
//!
//! Reused from the parity bot (recorded in the report): `parity::Client`
//! (buffered socket, keep-alive echo, compression, trace+stamps) and
//! `parity::join_steps_with_view` (the scripted join). The bot runs in
//! metrics-only mode: it counts packets and sizes in memory and never writes
//! JSONL — the JSONL write was the measurement bottleneck.
//!
//! The bot answers everything a live client answers (teleport ack,
//! `chunk_batch_received`, keep-alive via the client) and sends
//! `client_tick_end` every 50 ms on the absolute grid the first teleport
//! ack anchors, exactly like `parity::walk`.

use data::PacketIds;
use parity::{Client, ParityEnv, Step};
use protocol::PacketKey;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use parity::walk as pw;

/// Serverbound keys the bench bot needs (same shape the walk bot declares).
const KEY_ACCEPT: PacketKey = PacketKey::serverbound(
    protocol::ProtocolState::Play,
    "minecraft:accept_teleportation",
    0x00,
);
/// `player_loaded`.
const KEY_PLAYER_LOADED: PacketKey = PacketKey::serverbound(
    protocol::ProtocolState::Play,
    "minecraft:player_loaded",
    0x2C,
);
/// `client_tick_end`.
const KEY_TICK_END: PacketKey = PacketKey::serverbound(
    protocol::ProtocolState::Play,
    "minecraft:client_tick_end",
    0x0D,
);
/// `move_player_pos`.
const KEY_MOVE_POS: PacketKey = PacketKey::serverbound(
    protocol::ProtocolState::Play,
    "minecraft:move_player_pos",
    0x1E,
);

/// Clientbound keys the bot folds.
const KEY_PLAYER_POSITION: PacketKey = PacketKey::clientbound(
    protocol::ProtocolState::Play,
    "minecraft:player_position",
    0x41,
);
/// `chunk_batch_finished`.
const KEY_BATCH_FINISHED: PacketKey = protocol::ChunkBatchFinished::KEY;
/// `level_chunk_with_light`.
const KEY_CHUNK: PacketKey = protocol::LevelChunkWithLight::KEY;
/// `set_chunk_cache_center` (SetCenterChunk).
const KEY_CENTER: PacketKey = protocol::SetCenterChunk::KEY;
/// `forget_level_chunk` (ForgetLevelChunk).
const KEY_FORGET: PacketKey = protocol::ForgetLevelChunk::KEY;

/// One tick: 20 TPS, the grid the walk bot uses.
const TICK: Duration = Duration::from_millis(50);
/// Delay between the first teleport ack and `player_loaded` (the live
/// client's measured order).
const LOADED_DELAY: Duration = Duration::from_millis(1001);

/// Join timeout: a server that does not answer in 15 s is down.
const WAIT: Duration = Duration::from_secs(15);

/// Metrics the bench bot keeps (no file, no base64).
#[derive(Debug, Default, Clone)]
pub struct BotStats {
    /// Chunks received (`level_chunk_with_light`).
    pub chunks: u64,
    /// Chunk payload bytes received.
    pub chunk_bytes: u64,
    /// `set_chunk_cache_center` count.
    pub centers: u64,
    /// `forget_level_chunk` count.
    pub forgets: u64,
    /// Movement packets sent.
    pub moves: u64,
    /// `client_tick_end` packets sent.
    pub tick_ends: u64,
    /// Milliseconds from the connection attempt to the first teleport ack.
    pub join_ms: Option<f64>,
    /// The acknowledged spawn.
    pub spawn: Option<[f64; 3]>,
    /// Maximum chunk-batch queue depth the bot observed (packets between
    /// two `chunk_batch_finished`).
    pub max_batch: u64,
    /// Finished batches seen.
    pub batches: u64,
    /// The live chunk set the bot holds at the end (every `forget` removed
    /// a coordinate; the runner compares it with `window(final centre)`).
    pub live: Vec<(i32, i32)>,
    /// The final cache centre the bot saw (`set_chunk_cache_center`).
    pub final_center: Option<(i32, i32)>,
    /// Diagnostic event log (first events, `BENCH_TRACE=1` only): one line
    /// per centre move and per forget, with the live-set size.
    pub event_log: Vec<String>,
    /// Deep sleep counter: how many times a read timed out with nothing
    /// received for 3 s (a silence probe; the bench never expects one).
    pub silences: u64,
}

/// What a bot's session ended with.
#[derive(Debug)]
pub enum BotStop {
    /// Finished its scripted walk and settle.
    Completed,
    /// The server closed (or errored): the reason.
    Closed(String),
    /// The join itself failed.
    Join(String),
}

/// One bot session: join, then play `mode` until it ends.
pub struct Bot {
    /// The parity client underneath (socket, trace, stamps).
    client: Client,
    /// Packet ids for name resolution and sends.
    ids: Arc<PacketIds>,
    /// Metrics accumulated so far.
    stats: BotStats,
    /// Anchored tick grid (from the first teleport ack).
    tick: Option<tokio::time::Instant>,
    /// `player_loaded` deadline, once.
    loaded: Option<tokio::time::Instant>,
    /// Whether `player_loaded` has gone out (moves may start).
    loaded_sent: bool,
    /// Current position (absolute, the move packet's own values).
    pos: [f64; 3],
    /// Movement steps sent so far.
    moved: u64,
    /// Chunks seen since the previous `chunk_batch_finished`.
    batch: u64,
    /// Chunk coordinates of the batch in flight (moved to `live` at close).
    pending: Vec<(i32, i32)>,
    /// The live set: chunks received in closed batches, minus forgets.
    live: BTreeSet<(i32, i32)>,
}

/// One bench bot session: joins, walks, ends.
pub struct BotPlan {
    /// Movement steps to send (one per tick after `player_loaded`).
    pub steps: u64,
    /// Blocks per tick (+x).
    pub step_len: f64,
    /// How long to stand after the last step.
    pub settle: Duration,
}

impl Clone for BotPlan {
    fn clone(&self) -> Self {
        Self {
            steps: self.steps,
            step_len: self.step_len,
            settle: self.settle,
        }
    }
}

impl Bot {
    /// Joins the server: handshake → login → configuration → play, and
    /// waits for the first teleport to acknowledge it.
    pub async fn join(
        env: &ParityEnv,
        ids: Arc<PacketIds>,
        port: u16,
        view_distance: i8,
    ) -> anyhow::Result<Self> {
        let mut client = Client::connect("bench", bench_addr(port), Arc::clone(&ids))
            .await
            .map_err(|e| anyhow::anyhow!("connect: {e}"))?;
        let steps = parity::join_steps_with_view(
            env,
            &ids,
            port,
            view_distance,
            parity::PROBE_WAIT_NO_PROBE,
        );
        client
            .run(&steps)
            .await
            .map_err(|e| anyhow::anyhow!("join: {e}"))?;
        // The scripted join ends after the last packet of the sequence;
        // the first teleport arrives right after — wait for it here.
        let mut bot = Self {
            client,
            ids: Arc::clone(&ids),
            stats: BotStats::default(),
            tick: None,
            loaded: None,
            loaded_sent: false,
            pos: [0.0, 64.0, 0.0],
            moved: 0,
            batch: 0,
            pending: Vec::new(),
            live: BTreeSet::new(),
        };
        // Fold until the teleport arrives; ack it.
        loop {
            let until = tokio::time::Instant::now() + WAIT;
            let Some((id, payload)) = bot
                .client
                .recv_packet(until)
                .await
                .map_err(|e| anyhow::anyhow!("join recv: {e}"))?
            else {
                anyhow::bail!("join: no teleport within {WAIT:?}");
            };
            let name = bot
                .client
                .trace()
                .packets
                .last()
                .and_then(|e| e.name.clone());
            if name.as_deref() == Some("minecraft:player_position")
                || id == ids.id(KEY_PLAYER_POSITION)
            {
                if let Some((spawn, ack)) = pw::teleport_ack(&payload) {
                    bot.stats.spawn = Some(spawn);
                    bot.pos = spawn;
                    bot.stats.join_ms = bot.join_ms();
                    bot.client
                        .send_packet(ids.id(KEY_ACCEPT), &ack)
                        .await
                        .map_err(|e| anyhow::anyhow!("send ack: {e}"))?;
                    let now = tokio::time::Instant::now();
                    bot.tick = Some(now);
                    bot.loaded = Some(now + LOADED_DELAY);
                    break;
                }
                anyhow::bail!("join: undecodable teleport ({} B)", payload.len());
            }
            bot.fold(id, &payload);
        }
        Ok(bot)
    }

    /// Monotonic ms since the connection attempt (for `join_ms`).
    fn join_ms(&self) -> Option<f64> {
        self.client
            .stamps()
            .last()
            .map(|d| d.as_secs_f64() * 1000.0)
    }

    /// Folds one received packet into the metrics (no file output).
    fn fold(&mut self, id: u32, payload: &[u8]) {
        let ids = &self.ids;
        if id == ids.id(KEY_CHUNK) {
            // Bytes as they cross (every packet here was really received).
            self.stats.chunk_bytes += payload.len() as u64;
            self.batch += 1;
            self.stats.max_batch = self.stats.max_batch.max(self.batch);
            // The chunk coordinates are the first two big-endian i32s of
            // the payload (LevelChunkWithLight layout).
            if payload.len() >= 8 {
                let x = i32::from_be_bytes(payload[0..4].try_into().expect("4 bytes"));
                let z = i32::from_be_bytes(payload[4..8].try_into().expect("4 bytes"));
                self.pending.push((x, z));
            }
            // The forgets of a window move can arrive before the batch that
            // carries the entering chunks closes; a forget that names a
            // chunk still in the pending batch is an orphan by definition
            // (it was never sent), so the forget applies to the live set
            // and, if present, cancels the pending entry (the chunk was
            // queued-then-dropped on the server; the client never saw it).
        } else if id == ids.id(KEY_BATCH_FINISHED) {
            // A finished batch means every chunk inside it was really
            // received: the live set grows by the batch's coordinates and
            // the received counter by the batch's chunk count.
            self.stats.chunks += self.batch;
            self.live.extend(self.pending.drain(..));
            self.batch = 0;
            self.stats.batches += 1;
        } else if id == ids.id(KEY_CENTER) {
            self.stats.centers += 1;
            if let Ok((x, used)) = protocol::decode_varint(payload) {
                if let Ok((z, _)) = protocol::decode_varint(&payload[used..]) {
                    self.stats.final_center = Some((x, z));
                }
            }
            self.trace(format!(
                "center #{:03} to {:?}: live={} pending={}",
                self.stats.centers,
                self.stats.final_center,
                self.live.len(),
                self.pending.len()
            ));
        } else if id == ids.id(KEY_FORGET) {
            // The packed ChunkPos (z in the high word, x in the low word)
            // names the coordinate the client must drop.
            if payload.len() >= 8 {
                let z = i32::from_be_bytes(payload[0..4].try_into().expect("4 bytes"));
                let x = i32::from_be_bytes(payload[4..8].try_into().expect("4 bytes"));
                self.live.remove(&(x, z));
                // A forget for a chunk still queued in this batch: the
                // server dropped it before sending (the client never saw
                // the chunk packet, only the forget). Cancel the pending
                // entry so the batch close does not add it back.
                self.pending.retain(|&cell| cell != (x, z));
                self.trace(format!(
                    "forget ({x}, {z}): live={} pending={}",
                    self.live.len(),
                    self.pending.len()
                ));
            }
            self.stats.forgets += 1;
        }
    }

    /// Appends one diagnostic line when `BENCH_TRACE=1` (max 3000 lines).
    fn trace(&mut self, line: String) {
        if std::env::var_os("BENCH_TRACE").is_some() && self.stats.event_log.len() < 3000 {
            self.stats.event_log.push(line);
        }
    }

    /// Plays the plan: tick grid (tick_end → move after it), answers
    /// everything a live client answers; ends after `plan.steps` moves and
    /// `plan.settle` idle, or when the server closes.
    pub async fn play(mut self, plan: BotPlan) -> (BotStats, BotStop) {
        let mut settle_deadline: Option<tokio::time::Instant> = None;
        loop {
            // Wake on the nearest of: tick grid, loaded timer, settle.
            let now = tokio::time::Instant::now();
            let mut wake = self.tick.unwrap_or(now).max(now);
            if let Some(at) = self.loaded {
                wake = wake.min(at);
            }
            if let Some(at) = settle_deadline {
                wake = wake.min(at);
            }
            if now < wake {
                tokio::time::sleep_until(wake).await;
            }
            // Read everything that has arrived (keep-alives are echoed on
            // the way in by the client itself).
            loop {
                let until = tokio::time::Instant::now() + Duration::from_millis(1);
                match self.client.recv_packet(until).await {
                    Ok(Some((id, payload))) => self.fold(id, &payload),
                    Ok(None) => break,
                    Err(error) => {
                        return (self.stats, BotStop::Closed(error.to_string()));
                    }
                }
            }
            // Tick grid: tick_end, then exactly one move after it.
            if let Some(at) = self.tick {
                let now = tokio::time::Instant::now();
                if now >= at {
                    let id = self.ids.id(KEY_TICK_END);
                    if let Err(error) = self.client.send_packet(id, &[]).await {
                        return (self.stats, BotStop::Closed(error.to_string()));
                    }
                    self.stats.tick_ends += 1;
                    let mut next = at + TICK;
                    while next <= tokio::time::Instant::now() {
                        next += TICK;
                    }
                    self.tick = Some(next);
                    if self.loaded_sent && self.moved < plan.steps && self.step(plan.step_len).await
                    {
                        self.moved += 1;
                        self.stats.moves += 1;
                        if self.moved == plan.steps {
                            settle_deadline = Some(tokio::time::Instant::now() + plan.settle);
                        }
                    }
                }
            }
            // player_loaded once.
            if let Some(at) = self.loaded {
                if tokio::time::Instant::now() >= at {
                    let id = self.ids.id(KEY_PLAYER_LOADED);
                    if let Err(error) = self.client.send_packet(id, &[]).await {
                        return (self.stats, BotStop::Closed(error.to_string()));
                    }
                    self.loaded = None;
                    self.loaded_sent = true;
                }
            }
            // Settle: the walk is over.
            if let Some(at) = settle_deadline {
                if tokio::time::Instant::now() >= at {
                    self.stats.live = self.live.iter().copied().collect();
                    return (self.stats, BotStop::Completed);
                }
            }
        }
    }

    /// Sends one +x step; returns whether a packet went out.
    async fn step(&mut self, len: f64) -> bool {
        self.pos[0] += len;
        let payload = move_payload(&self.pos, true);
        let id = self.ids.id(KEY_MOVE_POS);
        match self.client.send_packet(id, &payload).await {
            Ok(()) => true,
            Err(error) => {
                eprintln!("bench bot move failed: {error}");
                false
            }
        }
    }
}

/// `move_player_pos` payload: 3 big-endian doubles + the on-ground byte.
fn move_payload(p: &[f64; 3], on_ground: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(25);
    for value in p {
        out.extend_from_slice(&value.to_be_bytes());
    }
    out.push(u8::from(on_ground));
    out
}

/// The address the bench connects to (the in-process server binds
/// localhost; the port is whatever the runner bound).
fn bench_addr(port: u16) -> std::net::SocketAddr {
    std::net::SocketAddr::from(([127, 0, 0, 1], port))
}

/// The scripted join steps, re-exported for the runner (parity's own).
pub fn join_steps(env: &ParityEnv, ids: &PacketIds, port: u16, view_distance: i8) -> Vec<Step> {
    parity::join_steps_with_view(env, ids, port, view_distance, parity::PROBE_WAIT_NO_PROBE)
}
