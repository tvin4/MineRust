//! The play state: the join sequence, the keep-alive loop and the chunk
//! batches of the world.

mod chunk_pace;
mod join;
mod keep_alive;

#[cfg(test)]
mod tests;

use chunk_pace::ChunkPace;

use data::{GameData, PacketIds};
use protocol::{
    decode_frame, ChunkBatchFinished, ChunkBatchReceived, ChunkBatchStart, ClientInformation,
    ConfirmTeleportation, KeepAlive, LevelChunkWithLight, Position, ProtocolState, StatusInfo,
    MAX_STRING_CHARS, MOVE_PLAYER_POS, MOVE_PLAYER_POS_ROT, MOVE_PLAYER_ROT,
    MOVE_PLAYER_STATUS_ONLY, PLAY_PLUGIN_MESSAGE,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::oneshot;
use tokio::time::Instant;
use tracing::{debug, info, trace, warn};

use crate::error::ConnectionError;
use crate::frame::{read_tracked, write_tracked};
use crate::loader::{LoadError, Loader};

use super::Client;

use keep_alive::{keep_alive_millis, wait_for_deadline, KeepAliveWatch, KEEP_ALIVE_INTERVAL};

/// Identifier of the dimension the player joins.
const OVERWORLD: &str = "minecraft:overworld";

/// Registry that defines numeric dimension type ids in Login (play).
const DIMENSION_TYPE_REGISTRY: &str = "minecraft:dimension_type";

/// Entity id assigned to the single player of a connection.
const ENTITY_ID: i32 = 1;

/// View distance the server announces in Login (play) **and the largest
/// radius it ever sends**: the radius of a connection is the client's own
/// request capped by this server maximum (`send_radius`), so Login never
/// promises more than the server is willing to send.
// SOURCE: wiki, unverified: the vanilla value is 10 (read from the login
// recording by the parity test `login_fields`, it is not in the game data).
const VIEW_DISTANCE: i32 = 10;

/// Sea level sent in Login (play).
// SOURCE: wiki, unverified: vanilla value, not read from the game data.
const SEA_LEVEL: i32 = 63;

/// Spawn block of the empty world; the player teleports to its centre.
const EMPTY_SPAWN: Position = Position { x: 8, y: 64, z: 8 };

/// What the play loop knows about the world a connection joins.
///
/// The server reads both values from the save at startup (step 3.3e):
/// `spawn` is `Data.spawn.pos` of `level.dat`, `hashed_seed` is the hash
/// of `world_gen_settings.dat`'s `seed`. The default is the empty M2
/// world — centre block eight and no seed — so a server without a world
/// behaves exactly as before (parity slices included).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldInfo {
    /// Block the player joins at; the teleport target and the chunk cache
    /// centre are derived from it (`Set Center Chunk` of the join).
    pub spawn: Position,
    /// `hashed_seed` of Login (play): the seed's hash, zero without a world.
    pub hashed_seed: i64,
}

impl Default for WorldInfo {
    fn default() -> Self {
        Self {
            spawn: EMPTY_SPAWN,
            hashed_seed: 0,
        }
    }
}

/// Everything the play loop touches of the world a connection joins: the
/// pool that reads it and the values read from the save (step 3.3e).
/// Grouped into one argument so [`play_state`] stays readable.
pub(super) struct World {
    /// Chunk loader pool; `None` when the server runs without a world.
    pub(super) loader: Option<Loader>,
    /// Join values of the save; the default is the empty M2 world.
    pub(super) info: WorldInfo,
}

/// Teleport id of the initial join teleport; the client must confirm it
/// before movement packets are taken seriously.
// SOURCE: wiki, unverified: the id numbering and the confirmation rule are
// documented, never compared against a vanilla server.
const JOIN_TELEPORT_ID: i32 = 1;

/// Serves the play state: sends the join sequence and then keeps the
/// connection alive until the client goes away.
///
/// The join sequence is Login (play), Set Default Spawn Position, Player
/// Info Update, Set Center Chunk, Game Event (start waiting for chunks)
/// and the initial Synchronize Player Position. Afterwards the client's
/// packets are read on a background task so the periodic keep-alive can be
/// sent even while no packet arrives; serverbound packets that carry no
/// gameplay in M2 (keep-alive echo, teleport confirmation, settings,
/// plugin messages, movement) are logged and ignored.
///
/// Unlike the states before it, play runs without a global timeout.
///
/// Chunks (step 3.3c) are read by the loader pool, never here: this task
/// queues one batch read over the pool's bounded channel and awaits the
/// reply inside the `select!` loop below, so the keep-alive branch keeps
/// firing while disk reads and payload builds happen on pool threads.
// SOURCE: wiki, unverified: the join order and field values are documented,
// never compared with a vanilla server (see docs/SOURCES.md).
pub(super) async fn play_state<R, W>(
    reader: R,
    mut writer: W,
    game_data: &GameData,
    status: &StatusInfo,
    client: &Client,
    ids: &PacketIds,
    world: World,
) -> Result<(), ConnectionError>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin,
{
    let player_name = client.player_name.as_str();
    // Chunks go out inside this radius only: what the client asked for in
    // configuration, never more than Login (play) announced (part A, 6).
    let radius = send_radius(client.view_distance);
    info!(
        client_view_distance = ?client.view_distance,
        radius,
        player = %player_name,
        "chunk send radius chosen"
    );
    // Packet 1..6 of the join (module documentation) — recorded packet by
    // packet by the session trace as they go out.
    join::send_join(
        &mut writer,
        game_data,
        status,
        player_name,
        ids,
        &world.info,
    )
    .await?;

    // Queue the world read right away: from here on the pool owns disk I/O
    // and payload building, this task only selects on the reply channel.
    let mut loading = match &world.loader {
        Some(loader) => match loader.load_batch().await {
            Ok(pending) => {
                info!(player = %player_name, "chunk batch requested from the loader");
                Some(pending)
            }
            Err(err) => {
                warn!(%err, "chunk loader unavailable, no chunks will be sent");
                None
            }
        },
        None => {
            // The configuration refuses to start without a world (server
            // crate); this still fires for in-process servers started by
            // tests directly through `net::serve`.
            warn!(
                player = %player_name,
                "no chunk loader: this server sends no chunks at all"
            );
            None
        }
    };
    // The client's chunk cache centre lives around the spawn; used below to
    // flag any chunk that arrives from outside the announced radius.
    let (center_x, center_z) = (world.info.spawn.x >> 4, world.info.spawn.z >> 4);

    // Frames are read on a background task: reading must survive the
    // select-loop branches below (a cancelled frame read would drop bytes
    // it already consumed and desynchronise the stream), while the keep
    // alive must go out even when the client sends nothing.
    let (frame_tx, mut frame_rx) =
        tokio::sync::mpsc::channel::<Result<Vec<u8>, ConnectionError>>(16);
    tokio::spawn(async move {
        let mut reader = reader;
        loop {
            match read_tracked(&mut reader, ProtocolState::Play).await {
                Ok(body) => {
                    if frame_tx.send(Ok(body)).await.is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let _ = frame_tx.send(Err(err)).await;
                    break;
                }
            }
        }
    });

    let mut keep_alive =
        tokio::time::interval_at(Instant::now() + KEEP_ALIVE_INTERVAL, KEEP_ALIVE_INTERVAL);
    let mut watch = KeepAliveWatch::default();
    // Step 3.3d: no pace until the client names one — the join burst is
    // sent whole, as the vanilla recording frames it.
    let mut pace = ChunkPace::default();
    loop {
        tokio::select! {
            // The client must answer the outstanding keep-alive in time,
            // otherwise the connection ends (like a kick).
            _ = wait_for_deadline(watch.deadline()) => {
                return Err(ConnectionError::KeepAliveTimeout);
            }
            _ = keep_alive.tick() => {
                let packet = KeepAlive { id: keep_alive_millis() };
                write_tracked(
                    &mut writer,
                    ProtocolState::Play,
                    ids.id(KeepAlive::KEY),
                    &packet.encode_payload(),
                )
                .await?;
                watch.on_sent(Instant::now());
            }
            // The world read finished: the batch goes out here, in its own
            // branch, so nothing else of the loop waited for the disk.
            result = batch_reply(&mut loading), if loading.is_some() => {
                loading = None;
                match result {
                    Ok(Ok(chunks)) => {
                        info!(found = chunks.len(), "world read finished, chunks found");
                        write_chunk_batch(&mut writer, ids, chunks, &pace, center_x, center_z, radius)
                            .await?;
                    }
                    Ok(Err(err)) => warn!(%err, "world read failed, no chunks sent"),
                    Err(err) => warn!(%err, "chunk loader dropped the reply"),
                }
            }
            frame = frame_rx.recv() => {
                let body = match frame {
                    Some(Ok(body)) => body,
                    Some(Err(err)) => return Err(err),
                    // The reader finished without a pending error: the
                    // client is gone and the connection simply closes.
                    None => return Ok(()),
                };
                let (packet_id, mut payload) = decode_frame(&body)?;
                match packet_id {
                    id if id == ids.id(ConfirmTeleportation::KEY) => {
                        let confirm = ConfirmTeleportation::decode_payload(&mut payload)?;
                        payload.ensure_empty()?;
                        debug!(
                            teleport_id = confirm.teleport_id,
                            x = confirm.x,
                            y = confirm.y,
                            z = confirm.z,
                            "teleport confirmed"
                        );
                    }
                    id if id == ids.id(KeepAlive::SERVERBOUND_KEY) => {
                        let keep_alive = KeepAlive::decode_payload(&mut payload)?;
                        payload.ensure_empty()?;
                        debug!(keep_alive_id = keep_alive.id, "keep alive answered");
                        watch.on_acked();
                    }
                    // Step 3.3d: the client names the pace it wants the
                    // following bursts in; the connection ignores claims
                    // that are not a sane rate (chunk_pace keeps the old
                    // one instead).
                    id if id == ids.id(ChunkBatchReceived::KEY) => {
                        let ack = ChunkBatchReceived::decode_payload(&mut payload)?;
                        payload.ensure_empty()?;
                        // The pacing floats: the client names how many
                        // chunks per tick it wants in the next bursts.
                        info!(
                            chunks_per_tick = ack.chunks_per_tick,
                            "chunk_batch_received: pace chosen by the client"
                        );
                        pace.ack(ack.chunks_per_tick);
                    }
                    id if id == ids.id(ClientInformation::PLAY_KEY) => {
                        let info = ClientInformation::decode_payload(&mut payload)?;
                        payload.ensure_empty()?;
                        debug!(
                            locale = %info.locale,
                            view_distance = info.view_distance,
                            "client information received"
                        );
                    }
                    id if id == ids.id(PLAY_PLUGIN_MESSAGE) => {
                        // The channel name is worth logging; M2 ignores the
                        // payload of plugin messages.
                        let channel = payload.read_string(MAX_STRING_CHARS)?;
                        debug!(%channel, "plugin message received, ignored");
                    }
                    // Movement carries gameplay the empty world does not
                    // simulate yet.
                    id
                        if id == ids.id(MOVE_PLAYER_POS)
                            || id == ids.id(MOVE_PLAYER_POS_ROT)
                            || id == ids.id(MOVE_PLAYER_ROT)
                            || id == ids.id(MOVE_PLAYER_STATUS_ONLY) =>
                    {
                        trace!(id, "movement received, ignored");
                    }
                    // M2 has no gameplay: unknown or not-yet-handled
                    // serverbound packets must not drop the connection.
                    id => {
                        debug!(id, "ignoring serverbound play packet");
                    }
                }
            }
        }
    }
}

/// Waits for the loader's reply on its one-shot channel.
///
/// While nothing is queued the receiver is `None` and this future never
/// resolves — the `select!` arm is guarded by the same option, this only
/// keeps the branch non-empty (the same shape as `wait_for_deadline`).
async fn batch_reply(
    loading: &mut Option<oneshot::Receiver<Result<Vec<LevelChunkWithLight>, LoadError>>>,
) -> Result<Result<Vec<LevelChunkWithLight>, LoadError>, oneshot::error::RecvError> {
    match loading {
        Some(pending) => pending.await,
        None => std::future::pending().await,
    }
}

/// Sends one chunk burst as the recording frames it: `chunk_batch_start`,
/// one `level_chunk_with_light` per payload, `chunk_batch_finished` with
/// the number of chunks in between.
///
/// With a pace the client confirmed (step 3.3d) the burst is cut into
/// sub-batches of that size, each framed identically and sent one game
/// tick apart; without one the burst goes out in a single piece, as the
/// recording shows it. An empty read keeps its empty start/finished pair.
///
/// SOURCE: parity trace of vanilla 26.3 — one start marker, nine payloads
/// and a finished marker whose VarInt equals the payload count (walked by
/// the parity test `blob_walk`; the recorded order of the burst itself).
async fn write_chunk_batch<W>(
    writer: &mut W,
    ids: &PacketIds,
    mut chunks: Vec<LevelChunkWithLight>,
    pace: &ChunkPace,
    center_x: i32,
    center_z: i32,
    radius: i32,
) -> Result<(), ConnectionError>
where
    W: AsyncWrite + Unpin,
{
    // The send gate (part A, 6): only chunks of the ready rule (the loader
    // already refused the rest) *inside* the radius go out. The client
    // drops a chunk outside its own view square, so sending one wastes a
    // payload and desynchronises the count of the batch framing.
    let before = chunks.len();
    chunks.retain(|chunk| !outside_view(chunk.chunk_x, chunk.chunk_z, center_x, center_z, radius));
    let outside = before - chunks.len();
    if outside > 0 {
        info!(
            radius,
            center_x,
            center_z,
            outside,
            inside = chunks.len(),
            "chunks outside the send radius are not sent"
        );
    }
    let per_batch = pace
        .sub_batch_size(chunks.len())
        .unwrap_or_else(|| chunks.len().max(1));
    let mut left = chunks.into_iter();
    let mut total = 0_i32;
    let mut batches = 0_u32;
    loop {
        let group: Vec<_> = left.by_ref().take(per_batch).collect();
        if group.is_empty() {
            if batches == 0 {
                // An empty read still frames its own (empty) batch.
                write_sub_batch(writer, ids, Vec::new(), center_x, center_z).await?;
            }
            info!(
                chunk_count = total,
                sub_batches = batches.max(1),
                "chunk batch sent"
            );
            return Ok(());
        }
        if batches > 0 {
            // One sub-batch per game tick of the client's own pace; the
            // pause only holds this branch — the keep-alive interval and
            // the deadline tick of their own tasks meanwhile.
            tokio::time::sleep(chunk_pace::TICK).await;
        }
        total += write_sub_batch(writer, ids, group, center_x, center_z).await?;
        batches += 1;
    }
}

/// Writes one `chunk_batch_start` … `chunk_batch_finished` pair around
/// `chunks`, with the finished count equal to what was sent in between.
///
/// Every chunk is logged with its coordinates and payload size; the batch
/// reaching here was already cut to the send radius by
/// [`write_chunk_batch`] (part A, 6), so every chunk fits the view square
/// around `center` the join announced.
async fn write_sub_batch<W>(
    writer: &mut W,
    ids: &PacketIds,
    chunks: Vec<LevelChunkWithLight>,
    center_x: i32,
    center_z: i32,
) -> Result<i32, ConnectionError>
where
    W: AsyncWrite + Unpin,
{
    debug!(center_x, center_z, "chunk_batch_start sent");
    write_tracked(
        writer,
        ProtocolState::Play,
        ids.id(ChunkBatchStart::KEY),
        &ChunkBatchStart.encode_payload(),
    )
    .await?;
    let mut chunk_count = 0_i32;
    for chunk in chunks {
        let payload = chunk.encode_payload()?;
        info!(
            chunk_x = chunk.chunk_x,
            chunk_z = chunk.chunk_z,
            size = payload.len(),
            "level_chunk_with_light sent"
        );
        write_tracked(
            writer,
            ProtocolState::Play,
            ids.id(LevelChunkWithLight::KEY),
            &payload,
        )
        .await?;
        chunk_count += 1;
    }
    let finished = ChunkBatchFinished { chunk_count };
    write_tracked(
        writer,
        ProtocolState::Play,
        ids.id(ChunkBatchFinished::KEY),
        &finished.encode_payload(),
    )
    .await?;
    info!(chunk_count, "chunk_batch_finished sent");
    Ok(chunk_count)
}

/// Whether a chunk lies outside the square of `radius` around `center`
/// (the cache centre the join announced in Set Center Chunk).
fn outside_view(chunk_x: i32, chunk_z: i32, center_x: i32, center_z: i32, radius: i32) -> bool {
    (chunk_x - center_x).abs() > radius || (chunk_z - center_z).abs() > radius
}

/// The radius this connection sends chunks in (the owner's rule, part A 6):
/// the view distance the client asked for in configuration, capped by the
/// maximum Login (play) announced. A client that asked for nothing — or
/// for something impossible (a negative or absurdly large value) — gets
/// the maximum the server announced, never less than nothing and never
/// more than [`VIEW_DISTANCE`].
fn send_radius(client_view_distance: Option<i32>) -> i32 {
    client_view_distance.map_or(VIEW_DISTANCE, |requested| requested.clamp(0, VIEW_DISTANCE))
}

#[cfg(test)]
mod send_radius_tests {
    use super::send_radius;

    #[test]
    fn the_radius_is_the_client_request_capped_by_the_server_maximum() {
        assert_eq!(send_radius(Some(8)), 8, "the bot's own request");
        assert_eq!(send_radius(Some(10)), 10, "the announced maximum");
        assert_eq!(send_radius(Some(16)), 10, "a larger request is capped");
        assert_eq!(send_radius(Some(32)), 10, "the client maximum is capped");
    }

    #[test]
    fn a_missing_or_impossible_request_falls_back_safely() {
        assert_eq!(send_radius(None), 10, "no request: the announced maximum");
        assert_eq!(send_radius(Some(0)), 0, "never below what the client asks");
        assert_eq!(send_radius(Some(-5)), 0, "an absurd request clamps down");
    }
}
