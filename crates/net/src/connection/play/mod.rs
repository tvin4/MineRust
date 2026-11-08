//! The play state: the join sequence, the keep-alive loop and the chunk
//! batches of the world.

mod keep_alive;

#[cfg(test)]
mod tests;

use data::{GameData, PacketIds};
use protocol::{
    decode_frame, offline_uuid, ChunkBatchFinished, ChunkBatchStart, ClientInformation,
    ConfirmTeleportation, GameEvent, KeepAlive, LevelChunkWithLight, LoginPlay, PlayerInfoEntry,
    PlayerInfoUpdate, Position, SetCenterChunk, SetDefaultSpawnPosition, StatusInfo,
    SynchronizePlayerPosition, MAX_STRING_CHARS, MOVE_PLAYER_POS, MOVE_PLAYER_POS_ROT,
    MOVE_PLAYER_ROT, MOVE_PLAYER_STATUS_ONLY, PLAY_PLUGIN_MESSAGE,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::oneshot;
use tokio::time::Instant;
use tracing::{debug, info, trace, warn};

use crate::error::ConnectionError;
use crate::frame::{read_frame, write_frame};
use crate::loader::{LoadError, Loader};

use keep_alive::{keep_alive_millis, wait_for_deadline, KeepAliveWatch, KEEP_ALIVE_INTERVAL};

/// Identifier of the dimension the player joins.
const OVERWORLD: &str = "minecraft:overworld";

/// Registry that defines numeric dimension type ids in Login (play).
const DIMENSION_TYPE_REGISTRY: &str = "minecraft:dimension_type";

/// Entity id assigned to the single player of a connection.
const ENTITY_ID: i32 = 1;

/// Render and simulation distance sent in Login (play), in chunks.
// SOURCE: wiki, unverified: vanilla values, not read from the game data.
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
    player_name: &str,
    ids: &PacketIds,
    world: World,
) -> Result<(), ConnectionError>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin,
{
    // Numeric dimension type id of the world the player joins. The empty
    // game data of tests has no registries, so fall back with a warning.
    let dimension_type = match game_data
        .registry(DIMENSION_TYPE_REGISTRY)
        .and_then(|registry| registry.entry_id(OVERWORLD))
    {
        Some(id) => id,
        None => {
            warn!(
                registry = DIMENSION_TYPE_REGISTRY,
                entry = OVERWORLD,
                "dimension type entry missing, falling back to id 0"
            );
            0
        }
    };
    // The dimension list carries the world's dimensions, not the dimension
    // types of the registry: the vanilla join recording lists exactly the
    // three dimensions of its own world and omits `minecraft:overworld_caves`,
    // which the registry does contain.
    // SOURCE: half-verified (the recording proves "not the registry"; the
    // world itself arrives with the world template in part 3). Our world is
    // the overworld alone for now.
    let dimension_names = vec![OVERWORLD.to_owned()];

    let login = LoginPlay {
        entity_id: ENTITY_ID,
        hardcore: false,
        dimension_names,
        max_players: status.max_players,
        view_distance: VIEW_DISTANCE,
        simulation_distance: VIEW_DISTANCE,
        reduced_debug_info: false,
        enable_respawn_screen: true,
        do_limited_crafting: false,
        dimension_type,
        dimension_name: OVERWORLD.to_owned(),
        // Step 3.3e: `world_gen_settings.dat`'s seed, hashed by the
        // formula the parity recording verified (test `login_fields`);
        // without a world the default stays zero.
        hashed_seed: world.info.hashed_seed,
        game_mode: 0,
        previous_game_mode: 0,
        is_debug: false,
        is_flat: false,
        death_location: None,
        portal_cooldown: 0,
        sea_level: SEA_LEVEL,
        online_mode: false,
        enforces_secure_chat: false,
    };
    write_frame(
        &mut writer,
        ids.id(LoginPlay::KEY),
        &login.encode_payload()?,
    )
    .await?;

    // Step 3.3e: the spawn of the save (`level.dat`) is where the player
    // joins — it also drives the chunk cache centre below, which vanilla
    // takes from the player's position.
    let spawn = SetDefaultSpawnPosition {
        dimension: OVERWORLD.to_owned(),
        position: world.info.spawn,
        yaw: 0.0,
        pitch: 0.0,
    };
    write_frame(
        &mut writer,
        ids.id(SetDefaultSpawnPosition::KEY),
        &spawn.encode_payload()?,
    )
    .await?;

    let uuid = offline_uuid(player_name);
    let info = PlayerInfoUpdate {
        players: vec![PlayerInfoEntry {
            uuid,
            name: player_name.to_owned(),
            properties: Vec::new(),
            game_mode: 0,
            listed: true,
            latency: 0,
        }],
    };
    write_frame(
        &mut writer,
        ids.id(PlayerInfoUpdate::KEY),
        &info.encode_payload()?,
    )
    .await?;

    // Centre of the cache: the position the player joined at (`spawn`),
    // expressed in chunks — vanilla keeps its view centred on the player.
    let center = SetCenterChunk {
        chunk_x: world.info.spawn.x >> 4,
        chunk_z: world.info.spawn.z >> 4,
    };
    write_frame(
        &mut writer,
        ids.id(SetCenterChunk::KEY),
        &center.encode_payload(),
    )
    .await?;

    // Without event 13 the client stays on its loading screen.
    // SOURCE: wiki, unverified: the event id and the "close the loading
    // screen" effect were observed as documented, not compared to vanilla.
    let event = GameEvent {
        event: GameEvent::START_WAITING_FOR_CHUNKS,
        value: 0.0,
    };
    write_frame(&mut writer, ids.id(GameEvent::KEY), &event.encode_payload()).await?;

    let teleport = SynchronizePlayerPosition {
        teleport_id: JOIN_TELEPORT_ID,
        x: f64::from(world.info.spawn.x) + 0.5,
        y: f64::from(world.info.spawn.y),
        z: f64::from(world.info.spawn.z) + 0.5,
        velocity_x: 0.0,
        velocity_y: 0.0,
        velocity_z: 0.0,
        yaw: 0.0,
        pitch: 0.0,
        flags: 0, // fully absolute teleport
    };
    write_frame(
        &mut writer,
        ids.id(SynchronizePlayerPosition::KEY),
        &teleport.encode_payload(),
    )
    .await?;
    info!(player = %player_name, entity_id = ENTITY_ID, "player joined");

    // Queue the world read right away: from here on the pool owns disk I/O
    // and payload building, this task only selects on the reply channel.
    let mut loading = match &world.loader {
        Some(loader) => match loader.load_batch().await {
            Ok(pending) => Some(pending),
            Err(err) => {
                warn!(%err, "chunk loader unavailable, no chunks will be sent");
                None
            }
        },
        None => None,
    };

    // Frames are read on a background task: reading must survive the
    // select-loop branches below (a cancelled frame read would drop bytes
    // it already consumed and desynchronise the stream), while the keep
    // alive must go out even when the client sends nothing.
    let (frame_tx, mut frame_rx) =
        tokio::sync::mpsc::channel::<Result<Vec<u8>, ConnectionError>>(16);
    tokio::spawn(async move {
        let mut reader = reader;
        loop {
            match read_frame(&mut reader).await {
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
    loop {
        tokio::select! {
            // The client must answer the outstanding keep-alive in time,
            // otherwise the connection ends (like a kick).
            _ = wait_for_deadline(watch.deadline()) => {
                return Err(ConnectionError::KeepAliveTimeout);
            }
            _ = keep_alive.tick() => {
                let packet = KeepAlive { id: keep_alive_millis() };
                write_frame(&mut writer, ids.id(KeepAlive::KEY), &packet.encode_payload()).await?;
                watch.on_sent(Instant::now());
            }
            // The world read finished: the batch goes out here, in its own
            // branch, so nothing else of the loop waited for the disk.
            result = batch_reply(&mut loading), if loading.is_some() => {
                loading = None;
                match result {
                    Ok(Ok(chunks)) => write_chunk_batch(&mut writer, ids, chunks).await?,
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
/// SOURCE: parity trace of vanilla 26.3 — one start marker, nine payloads
/// and a finished marker whose VarInt equals the payload count (walked by
/// the parity test `blob_walk`; the recorded order of the burst itself).
async fn write_chunk_batch<W>(
    writer: &mut W,
    ids: &PacketIds,
    chunks: Vec<LevelChunkWithLight>,
) -> Result<(), ConnectionError>
where
    W: AsyncWrite + Unpin,
{
    write_frame(
        writer,
        ids.id(ChunkBatchStart::KEY),
        &ChunkBatchStart.encode_payload(),
    )
    .await?;
    let mut chunk_count = 0_i32;
    for chunk in chunks {
        let payload = chunk.encode_payload()?;
        write_frame(writer, ids.id(LevelChunkWithLight::KEY), &payload).await?;
        chunk_count += 1;
    }
    let finished = ChunkBatchFinished { chunk_count };
    write_frame(
        writer,
        ids.id(ChunkBatchFinished::KEY),
        &finished.encode_payload(),
    )
    .await?;
    debug!(chunk_count, "chunk batch sent");
    Ok(())
}
