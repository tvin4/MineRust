//! The play state: the join sequence, the keep-alive loop and the chunk
//! batches of the world.

mod batch;
mod chunk_pace;
mod join;
mod keep_alive;

#[cfg(test)]
mod tests;

#[cfg(test)]
use batch::{sort_by_distance, write_chunk_batch};
use batch::{wait_turn, PendingBatch};
use chunk_pace::ChunkPace;

use data::{GameData, PacketIds};
use protocol::{
    decode_frame, ChunkBatchReceived, ClientInformation, ConfirmTeleportation, KeepAlive,
    PacketReader, Position, ProtocolState, StatusInfo, MAX_STRING_CHARS, MOVE_PLAYER_POS,
    MOVE_PLAYER_POS_ROT, MOVE_PLAYER_ROT, MOVE_PLAYER_STATUS_ONLY, PLAY_PLUGIN_MESSAGE,
};
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::oneshot;
use tokio::time::Instant;
use tracing::{debug, info, trace, warn};

use crate::chunks::ReadyChunks;
use crate::error::ConnectionError;
use crate::frame::{read_tracked, write_tracked};
use crate::game::{
    GameHandle, KickFlag, MoveKind, MoveSlot, MoveState, PlayerConnection, PlayerId, PlayerJoin,
    ServerMessage, ToGame, OUTBOUND_CAPACITY,
};
use crate::loader::{LoadError, Loader};

use super::Client;

use keep_alive::{keep_alive_millis, wait_for_deadline, KeepAliveWatch, KEEP_ALIVE_INTERVAL};

/// Identifier of the dimension the player joins.
const OVERWORLD: &str = "minecraft:overworld";

/// Registry that defines numeric dimension type ids in Login (play).
const DIMENSION_TYPE_REGISTRY: &str = "minecraft:dimension_type";

/// Sea level sent in Login (play).
// SOURCE: wiki, unverified: vanilla value, not read from the game data.
const SEA_LEVEL: i32 = 63;

/// Both distances Login (play) announces when no config was passed (the
/// parity slices and the unit tests build a server without `server.toml`):
/// the same 10/10 the real server's `server.toml` fields default to
/// (`server::DEFAULT_DISTANCES`).
const DEFAULT_DISTANCES: i32 = 10;

/// Spawn block of the empty world; the player teleports to its centre.
const EMPTY_SPAWN: Position = Position { x: 8, y: 64, z: 8 };

/// What the play loop knows about the world a connection joins.
///
/// `spawn` and `hashed_seed` are read from the save at startup (step
/// 3.3e): `spawn` is `Data.spawn.pos` of `level.dat`, `hashed_seed` is
/// the hash of `world_gen_settings.dat`'s `seed`; `view_distance` and
/// `simulation_distance` come from `server.toml` (both default to
/// [`DEFAULT_DISTANCES`]) and are announced in Login (play). The default
/// is the empty M2
/// world — centre block eight and no seed — so a server without a world
/// behaves exactly as before (parity slices included).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorldInfo {
    /// Block the player joins at; the teleport target and the chunk cache
    /// centre are derived from it (`Set Center Chunk` of the join).
    pub spawn: Position,
    /// `hashed_seed` of Login (play): the seed's hash, zero without a world.
    pub hashed_seed: i64,
    /// View distance announced in Login (play) (`server.toml`,
    /// `view_distance`) **and the largest radius this server ever sends**:
    /// the radius of a connection is the client's own request capped by
    /// this maximum (`send_radius`), so Login never promises more than the
    /// server is willing to send.
    // SOURCE: the parity test `login_fields` reads 10 out of the vanilla
    // login; 10 is also what `server.toml` defaults the field to.
    pub view_distance: i32,
    /// Simulation distance announced in Login (play) (`server.toml`,
    /// `simulation_distance`). Nothing is simulated yet — the field only
    /// announces the number (M4 and later use it).
    // SOURCE: the parity test `login_fields` reads 10 out of the vanilla
    // login; 10 is also what `server.toml` defaults the field to.
    pub simulation_distance: i32,
}

impl Default for WorldInfo {
    fn default() -> Self {
        Self {
            spawn: EMPTY_SPAWN,
            hashed_seed: 0,
            view_distance: DEFAULT_DISTANCES,
            simulation_distance: DEFAULT_DISTANCES,
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

/// The immutable inputs of one play session, grouped so [`play_state`]
/// stays under clippy's argument limit.
pub(super) struct Session<'a> {
    /// Registries and tags the join packets read.
    pub(super) game_data: &'a GameData,
    /// Status document (max players of Login (play)).
    pub(super) status: &'a StatusInfo,
    /// What login and configuration learned about the client.
    pub(super) client: &'a Client,
    /// Numeric packet ids.
    pub(super) ids: &'a PacketIds,
    /// The world this connection joins.
    pub(super) world: World,
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
/// sent even while no packet arrives; keep-alive echo and teleport
/// confirmation are handled, the newest movement packet goes into the
/// game thread's slot, and everything else is logged and ignored.
///
/// The player's game state (who is online, whether the entry teleport was
/// confirmed, the newest movement) lives on the game thread: this task only
/// sends the reliable `Join`/`TeleportAck`/`Quit` upward and writes back
/// whatever the tick asks for (M4.1: nothing yet).
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
    session: Session<'_>,
    game: GameHandle,
) -> Result<(), ConnectionError>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin,
{
    let Session {
        game_data,
        status,
        client,
        ids,
        world,
    } = session;
    let player_name = client.player_name.as_str();
    // Chunks go out inside this radius only: what the client asked for in
    // configuration, never more than Login (play) announced (part A, 6).
    let radius = send_radius(client.view_distance, world.info.view_distance);
    // The entity id of Login (play) comes from the game thread's counter:
    // the first player of a server keeps the id the join always announced.
    let player_id = game.next_player_id();
    info!(
        client_view_distance = ?client.view_distance,
        server_view_distance = world.info.view_distance,
        radius,
        player = %player_name,
        %player_id,
        "chunk send radius chosen"
    );
    // Packet 1..6 of the join (module documentation) — recorded packet by
    // packet by the session trace as they go out.
    join::send_join(
        &mut writer,
        game_data,
        status,
        player_name,
        player_id,
        ids,
        &world.info,
    )
    .await?;
    // The connection's outbound queue, the kick flag and the movement slot
    // are all created here and half of each goes to the game thread.
    let (outbound, mut outbound_rx) =
        tokio::sync::mpsc::channel::<ServerMessage>(OUTBOUND_CAPACITY);
    let kick = Arc::new(KickFlag::default());
    let last_move = Arc::new(MoveSlot::default());
    let join = PlayerJoin {
        id: player_id,
        name: player_name.to_owned(),
        view_distance: radius,
        spawn: world.info.spawn,
        connection: PlayerConnection {
            outbound,
            kick: Arc::clone(&kick),
            last_move: Arc::clone(&last_move),
        },
    };
    if let Err(err) = game.send(ToGame::Join(Box::new(join))).await {
        warn!(%err, player = %player_name, "the game thread is gone, the player is not registered");
    }
    // Unregisters the player on every exit of this task (error, disconnect
    // or panic) without needing an unwind-safe async path.
    let _guard = PlayerGuard {
        game: game.clone(),
        id: player_id,
    };

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
    // Step 3.3d: no pace until the client names one — the burst opens
    // with its first flush instead of going out whole (batch module: the
    // recorded vanilla burst is 9 of 72 chunks before the answer).
    let mut pace = ChunkPace::default();
    // The burst on its way: written one sub-batch per turn of this loop,
    // never inside one branch — the reader stays serviceable between two
    // sub-batches, so a keep-alive is answered and the pace answer lands
    // while the burst is still going (the sub_batches=1 symptom: writing
    // the whole burst in the loading branch folded the read side into the
    // write, and `chunk_batch_received` arrived only after it).
    let mut pending: Option<PendingBatch> = None;
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
                        pending = Some(PendingBatch::new(chunks, center_x, center_z, radius));
                    }
                    Ok(Err(err)) => warn!(%err, "world read failed, no chunks sent"),
                    Err(err) => warn!(%err, "chunk loader dropped the reply"),
                }
            }
            // One sub-batch of the burst — the first flush right away, the
            // paced tail one game tick after the previous one, and after
            // the first flush only once the pace answer (or its guard
            // deadline) is in.
            turn = wait_turn(pending.as_ref().map(PendingBatch::stage)), if pending.is_some() => {
                let Some(batch) = pending.as_mut() else {
                    // The arm is guarded by `is_some()`; keep the branch
                    // exhaustive instead of an unreachable assumption.
                    continue;
                };
                debug_assert!(turn.is_some(), "a pending burst always has a stage");
                if batch.send_next(&mut writer, ids, &pace).await? {
                    pending = None;
                }
            }
            // The game thread asked this connection to send a packet.
            // M4.1's tick sends none; the arm keeps the path ready and
            // notices the end of the queue (a quit or a kick).
            message = outbound_rx.recv() => {
                match message {
                    Some(ServerMessage::Send { id, payload }) => {
                        write_tracked(&mut writer, ProtocolState::Play, id, &payload).await?;
                    }
                    // The player's window moved (M4.4 B). ШАГ 3 turns this
                    // into `set_chunk_cache_center`, `forget_level_chunk` for
                    // the leaving chunks and a new paced batch for the
                    // entering ones; until then the event is only logged.
                    Some(ServerMessage::StreamChunks {
                        center,
                        add,
                        remove,
                    }) => {
                        debug!(
                            center = ?center,
                            add = add.len(),
                            remove = remove.len(),
                            "window move (streaming lands in ШАГ 3)"
                        );
                    }
                    // The game thread dropped this player (quit); the
                    // connection closes. The kick reason is read in M4.1's
                    // kick policy (see the game module).
                    None => return Ok(()),
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
                        // The game thread only trusts movement after this.
                        if let Err(err) = game
                            .send(ToGame::TeleportAck {
                                id: player_id,
                                teleport_id: confirm.teleport_id,
                            })
                            .await
                        {
                            warn!(%err, "the game thread is gone, teleport ack dropped");
                        }
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
                        // A burst waiting for exactly this answer starts
                        // its tail at once (the vanilla trace sends the
                        // second flush right behind the first answer).
                        if let Some(batch) = pending.as_mut() {
                            batch.on_paced();
                        }
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
                    // Movement: the connection only classifies the packet
                    // (it matched the runtime id); the newest one goes into
                    // the player's slot and the game thread decodes and
                    // applies it in the tick (M4.2, ARCH-M4 §0).
                    id if id == ids.id(MOVE_PLAYER_POS) => {
                        store_move(&last_move, MoveKind::Pos, &mut payload, id)?;
                    }
                    id if id == ids.id(MOVE_PLAYER_POS_ROT) => {
                        store_move(&last_move, MoveKind::PosRot, &mut payload, id)?;
                    }
                    id if id == ids.id(MOVE_PLAYER_ROT) => {
                        store_move(&last_move, MoveKind::Rot, &mut payload, id)?;
                    }
                    id if id == ids.id(MOVE_PLAYER_STATUS_ONLY) => {
                        store_move(&last_move, MoveKind::StatusOnly, &mut payload, id)?;
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

/// Unregisters the player from the game thread when the play task ends.
///
/// Dropping the guard covers every exit (return, error or panic) without
/// an unwind-safe async path. The send is best-effort (`try_send`): a full
/// game queue drops the quit, which only leaves a stale player until the
/// connection's outbound queue is observed closed.
struct PlayerGuard {
    game: GameHandle,
    id: PlayerId,
}

impl Drop for PlayerGuard {
    fn drop(&mut self) {
        self.game.quit_now(self.id);
    }
}

/// Copies the newest movement payload into the player's slot.
///
/// The connection classifies the packet and keeps the bytes; the game thread
/// decodes them in the tick (M4.2). The whole payload is copied as-is, so a
/// malformed one is caught by the decoder, not here.
fn store_move(
    slot: &MoveSlot,
    kind: MoveKind,
    payload: &mut PacketReader<'_>,
    id: u32,
) -> Result<(), ConnectionError> {
    let length = payload.remaining();
    let bytes = payload.read_bytes(length)?.to_vec();
    slot.store(MoveState {
        kind,
        payload: bytes,
    });
    trace!(id, ?kind, length, "movement stored in the last-move slot");
    Ok(())
}

/// The one-shot answer of the loader pool: the shared ready world of a
/// connection or the error of its read. Both sides of the pool move the
/// same `Arc<ReadyChunks>` (part A 2 — a connection clones the pointer,
/// never the payloads), so the branches name one type instead of the full
/// nesting.
type BatchReply = Result<Arc<ReadyChunks>, LoadError>;
/// Waits for the loader's reply on its one-shot channel.
///
/// While nothing is queued the receiver is `None` and this future never
/// resolves — the `select!` arm is guarded by the same option, this only
/// keeps the branch non-empty (the same shape as `wait_for_deadline`).
async fn batch_reply(
    loading: &mut Option<oneshot::Receiver<BatchReply>>,
) -> Result<BatchReply, oneshot::error::RecvError> {
    match loading {
        Some(pending) => pending.await,
        None => std::future::pending().await,
    }
}

/// The radius this connection sends chunks in (the owner's rule, part A 6):
/// the view distance the client asked for in configuration, capped by the
/// maximum Login (play) announced — the first is the client's own
/// `client_information`, the second is the server's `view_distance`
/// (`server.toml`, default [`DEFAULT_DISTANCES`]). A client that asked for
/// nothing — or for something impossible (a negative or absurdly large
/// value) — gets the maximum the server announced, never less than nothing
/// and never more than it.
fn send_radius(client_view_distance: Option<i32>, server_view_distance: i32) -> i32 {
    client_view_distance.map_or(server_view_distance, |requested| {
        requested.clamp(0, server_view_distance)
    })
}

#[cfg(test)]
mod send_radius_tests {
    use super::{send_radius, DEFAULT_DISTANCES};

    #[test]
    fn the_radius_is_the_client_request_capped_by_the_server_maximum() {
        assert_eq!(send_radius(Some(8), 10), 8, "the bot's own request");
        assert_eq!(send_radius(Some(10), 10), 10, "the announced maximum");
        assert_eq!(send_radius(Some(16), 10), 10, "a larger request is capped");
        assert_eq!(
            send_radius(Some(32), 10),
            10,
            "the client maximum is capped"
        );
    }

    #[test]
    fn the_configured_server_view_distance_caps_its_own() {
        assert_eq!(send_radius(None, 6), 6, "no request: the configured one");
        assert_eq!(send_radius(Some(8), 6), 6, "a larger request is capped");
        assert_eq!(send_radius(Some(3), 6), 3, "a smaller one stands");
    }

    #[test]
    fn a_missing_or_impossible_request_falls_back_safely() {
        assert_eq!(
            send_radius(None, DEFAULT_DISTANCES),
            10,
            "no request: the announced maximum"
        );
        assert_eq!(
            send_radius(Some(0), 10),
            0,
            "never below what the client asks"
        );
        assert_eq!(
            send_radius(Some(-5), 10),
            0,
            "an absurd request clamps down"
        );
    }
}
