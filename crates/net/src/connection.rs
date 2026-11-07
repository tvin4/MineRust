//! Connection handling: handshake parsing and the status, login,
//! configuration and play state machines.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use data::{GameData, PacketIds};
use protocol::{
    decode_frame, offline_uuid, ClientInformation, ConfirmTeleportation, FinishAck,
    FinishConfiguration, GameEvent, GameProfile, Handshake, KeepAlive, LoginAcknowledged,
    LoginPlay, LoginStart, LoginSuccess, NextState, PingRequest, PlayerInfoEntry, PlayerInfoUpdate,
    PongResponse, Position, RegistryData, RegistryEntry, SetCenterChunk, SetDefaultSpawnPosition,
    StatusInfo, StatusRequest, StatusResponse, SynchronizePlayerPosition, UpdateTags,
    CONFIGURATION_PLUGIN_MESSAGE, MAX_STRING_CHARS, MOVE_PLAYER_POS, MOVE_PLAYER_POS_ROT,
    MOVE_PLAYER_ROT, MOVE_PLAYER_STATUS_ONLY, PLAY_PLUGIN_MESSAGE,
};
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;
use tracing::{debug, info, trace, warn};

use crate::error::ConnectionError;
use crate::frame::{read_frame, write_frame};

/// Maximum time a connection may spend before reaching the play state
/// (handshake, status, login and configuration). Once in play there is no
/// global timeout: a joined client is kept alive by keep-alives instead.
// SOURCE: wiki, unverified (the timeout/keep-alive numbers are our own,
// matching the wiki's suggested intervals; only a live client proves them).
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Identifier of the dimension the player joins.
const OVERWORLD: &str = "minecraft:overworld";

/// Registry that defines numeric dimension type ids for Login (play).
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
const SPAWN: Position = Position { x: 8, y: 64, z: 8 };

/// Teleport id of the initial join teleport; the client must confirm it
/// before movement packets are taken seriously.
// SOURCE: wiki, unverified: the id numbering and the confirmation rule are
// documented, never compared against a vanilla server.
const JOIN_TELEPORT_ID: i32 = 1;

/// How often the server asks the client for a keep-alive response in play.
// SOURCE: wiki, unverified (documented interval, exercised only by tests
// and by the parity crate once it exists).
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(10);

/// Accepts connections forever, spawning one task per connection.
///
/// The status document, the loaded game data and the packet ids are shared
/// as immutable `Arc`s; there is no mutable shared state on this path.
pub async fn serve(
    listener: TcpListener,
    status: Arc<StatusInfo>,
    game_data: Arc<GameData>,
    packet_ids: Arc<PacketIds>,
) -> std::io::Result<()> {
    loop {
        let (stream, peer) = listener.accept().await?;
        let status = Arc::clone(&status);
        let game_data = Arc::clone(&game_data);
        let packet_ids = Arc::clone(&packet_ids);
        tokio::spawn(async move {
            match handle_connection(stream, status, game_data, packet_ids).await {
                Ok(()) => debug!(%peer, "connection closed"),
                Err(ConnectionError::TimedOut) => debug!(%peer, "connection timed out"),
                Err(err) if err.is_client_disconnect() => {
                    debug!(%peer, %err, "client disconnected");
                }
                Err(err) => warn!(%peer, %err, "connection failed"),
            }
        });
    }
}

/// Runs a pre-play future under [`CONNECTION_TIMEOUT`].
async fn with_timeout<F, T>(future: F) -> Result<T, ConnectionError>
where
    F: std::future::Future<Output = Result<T, ConnectionError>>,
{
    match tokio::time::timeout(CONNECTION_TIMEOUT, future).await {
        Ok(result) => result,
        Err(_) => Err(ConnectionError::TimedOut),
    }
}

/// Drives one connection from the handshake into its target state.
///
/// Status and login (including configuration) run under
/// [`CONNECTION_TIMEOUT`]; the play state that follows is not capped.
async fn handle_connection(
    stream: TcpStream,
    status: Arc<StatusInfo>,
    game_data: Arc<GameData>,
    packet_ids: Arc<PacketIds>,
) -> Result<(), ConnectionError> {
    let ids = packet_ids.as_ref();
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = write_half;

    let body = with_timeout(read_frame(&mut reader)).await?;
    let (packet_id, mut payload) = decode_frame(&body)?;
    if packet_id != ids.id(Handshake::KEY) {
        return Err(ConnectionError::UnexpectedPacket {
            id: packet_id,
            state: "handshake",
        });
    }
    let handshake = Handshake::decode_payload(&mut payload)?;
    payload.ensure_empty()?;
    debug!(
        protocol = handshake.protocol_version,
        address = %handshake.server_address,
        port = handshake.server_port,
        "handshake received"
    );

    match handshake.next_state {
        NextState::Status => {
            with_timeout(status_state(&mut reader, &mut writer, &status, ids)).await
        }
        NextState::Login => {
            let player_name =
                with_timeout(login_state(&mut reader, &mut writer, &game_data, ids)).await?;
            play_state(reader, writer, &game_data, &status, &player_name, ids).await
        }
        // Transfer is a separate protocol flow not needed by M2.
        NextState::Transfer => {
            debug!("transfer state not implemented, closing connection");
            Ok(())
        }
    }
}

/// Serves the status state until the client disconnects or completes a ping.
async fn status_state<R, W>(
    reader: &mut R,
    writer: &mut W,
    status: &StatusInfo,
    ids: &PacketIds,
) -> Result<(), ConnectionError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    loop {
        let body = read_frame(reader).await?;
        let (packet_id, mut payload) = decode_frame(&body)?;
        match packet_id {
            // A client may also send a ping without a preceding request.
            id if id == ids.id(StatusRequest::KEY) => {
                StatusRequest::decode_payload(&mut payload)?;
                let response = StatusResponse {
                    json: status.to_json(),
                };
                write_frame(
                    writer,
                    ids.id(StatusResponse::KEY),
                    &response.encode_payload()?,
                )
                .await?;
            }
            id if id == ids.id(PingRequest::KEY) => {
                let ping = PingRequest::decode_payload(&mut payload)?;
                payload.ensure_empty()?;
                let pong = PongResponse {
                    payload: ping.payload,
                };
                write_frame(writer, ids.id(PongResponse::KEY), &pong.encode_payload()).await?;
                // The protocol says the connection closes after the pong.
                return Ok(());
            }
            id => {
                return Err(ConnectionError::UnexpectedPacket {
                    id,
                    state: "status",
                });
            }
        }
    }
}

/// Serves the login state: Login Start is answered with Login Success and
/// the client confirms with Login Acknowledged to reach configuration,
/// which must be completed before the player name is returned.
///
/// Runs under the pre-play timeout; see [`super::handle_connection`].
async fn login_state<R, W>(
    reader: &mut R,
    writer: &mut W,
    game_data: &GameData,
    ids: &PacketIds,
) -> Result<String, ConnectionError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let body = read_frame(reader).await?;
    let (packet_id, mut payload) = decode_frame(&body)?;
    if packet_id != ids.id(LoginStart::KEY) {
        return Err(ConnectionError::UnexpectedPacket {
            id: packet_id,
            state: "login",
        });
    }
    let start = LoginStart::decode_payload(&mut payload)?;
    payload.ensure_empty()?;
    debug!(name = %start.name, "login start received");

    // Offline login: no encryption, no compression, and the profile UUID is
    // the documented offline derivation of the name.
    let uuid = offline_uuid(&start.name);
    let success = LoginSuccess {
        profile: GameProfile {
            uuid,
            name: start.name.clone(),
            properties: Vec::new(),
        },
        // A fresh random session UUID: the vanilla server draws a new one
        // per connection (two recorded join slices end login_finished with
        // two different version-4 UUIDs, while the first 41 bytes are the
        // same on both sides).
        // SOURCE: parity recording of vanilla 26.3 (behaviour-level).
        session_id: random_session_uuid()?,
    };
    write_frame(
        writer,
        ids.id(LoginSuccess::KEY),
        &success.encode_payload()?,
    )
    .await?;

    let body = read_frame(reader).await?;
    let (packet_id, mut payload) = decode_frame(&body)?;
    if packet_id != ids.id(LoginAcknowledged::KEY) {
        return Err(ConnectionError::UnexpectedPacket {
            id: packet_id,
            state: "login",
        });
    }
    LoginAcknowledged::decode_payload(&mut payload)?;
    payload.ensure_empty()?;
    debug!("login acknowledged, entering configuration");

    configuration_state(reader, writer, game_data, ids).await?;
    Ok(start.name)
}

/// Serves the configuration state: sends every synchronized registry, the
/// tags and Finish Configuration, then waits for the client's Finish
/// Acknowledged; afterwards the connection moves on to play.
async fn configuration_state<R, W>(
    reader: &mut R,
    writer: &mut W,
    game_data: &GameData,
    ids: &PacketIds,
) -> Result<(), ConnectionError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let registries = game_data.registries().count();
    for (registry_id, registry) in game_data.registries() {
        let packet = RegistryData {
            registry_id: registry_id.to_owned(),
            entries: registry
                .entries()
                .iter()
                .map(|entry| RegistryEntry {
                    name: entry.name.clone(),
                    data: Some(entry.data.clone()),
                })
                .collect(),
        };
        write_frame(writer, ids.id(RegistryData::KEY), &packet.encode_payload()?).await?;
    }

    let tags = game_data.tags_packet();
    write_frame(writer, ids.id(UpdateTags::KEY), &tags.encode_payload()?).await?;
    write_frame(
        writer,
        ids.id(FinishConfiguration::KEY),
        &FinishConfiguration.encode_payload(),
    )
    .await?;
    info!(
        registries,
        tag_registries = tags.registries.len(),
        "configuration data sent"
    );

    loop {
        let body = read_frame(reader).await?;
        let (packet_id, mut payload) = decode_frame(&body)?;
        match packet_id {
            id if id == ids.id(ClientInformation::KEY) => {
                let info = ClientInformation::decode_payload(&mut payload)?;
                payload.ensure_empty()?;
                debug!(
                    locale = %info.locale,
                    view_distance = info.view_distance,
                    "client information received"
                );
            }
            id if id == ids.id(CONFIGURATION_PLUGIN_MESSAGE) => {
                // The channel name is worth logging; M2 ignores the payload.
                let channel = payload.read_string(MAX_STRING_CHARS)?;
                debug!(%channel, "plugin message received, ignored");
            }
            id if id == ids.id(FinishAck::KEY) => {
                FinishAck::decode_payload(&mut payload)?;
                payload.ensure_empty()?;
                debug!("configuration acknowledged, entering play");
                return Ok(());
            }
            id => {
                return Err(ConnectionError::UnexpectedPacket {
                    id,
                    state: "configuration",
                });
            }
        }
    }
}

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
// SOURCE: wiki, unverified: the join order and field values are documented,
// never compared with a vanilla server (see docs/SOURCES.md).
async fn play_state<R, W>(
    reader: R,
    mut writer: W,
    game_data: &GameData,
    status: &StatusInfo,
    player_name: &str,
    ids: &PacketIds,
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
    let dimension_names: Vec<String> = game_data
        .registry(DIMENSION_TYPE_REGISTRY)
        .map(|registry| {
            registry
                .entries()
                .iter()
                .map(|entry| entry.name.clone())
                .collect()
        })
        .unwrap_or_default();

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
        hashed_seed: 0, // TODO(M3): hash the world seed once worldgen exists.
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

    let spawn = SetDefaultSpawnPosition {
        dimension: OVERWORLD.to_owned(),
        position: SPAWN,
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

    let center = SetCenterChunk {
        chunk_x: SPAWN.x >> 4,
        chunk_z: SPAWN.z >> 4,
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
        x: f64::from(SPAWN.x) + 0.5,
        y: f64::from(SPAWN.y),
        z: f64::from(SPAWN.z) + 0.5,
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
    loop {
        tokio::select! {
            _ = keep_alive.tick() => {
                let packet = KeepAlive { id: unix_millis() };
                write_frame(&mut writer, ids.id(KeepAlive::KEY), &packet.encode_payload()).await?;
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

/// Milliseconds since the Unix epoch, used as the keep-alive id.
fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| i64::try_from(elapsed.as_millis()).unwrap_or_default())
        .unwrap_or_default()
}

/// A random version-4 UUID for one connection's session id.
///
/// The vanilla 26.3 server sends a fresh random UUID in the tail of every
/// `minecraft:login_finished` recording (version nibble 4, RFC 4122
/// variant, changing between runs), so we draw our own per connection.
// SOURCE: parity recording of vanilla 26.3 (behaviour-level, two runs).
fn random_session_uuid() -> Result<[u8; 16], ConnectionError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|err| ConnectionError::Entropy(err.to_string()))?;
    bytes[6] = (bytes[6] & 0x0F) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3F) | 0x80; // RFC 4122 variant
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The session UUID must look like the vanilla one: version 4, RFC 4122
    /// variant, and never the same between two connections.
    #[test]
    fn random_session_uuid_is_a_fresh_version_four_id() {
        let first = random_session_uuid().expect("entropy available");
        let second = random_session_uuid().expect("entropy available");
        assert_ne!(first, second, "must not repeat within a run");
        assert_eq!(first[6] >> 4, 4, "version nibble must be 4");
        assert_eq!(first[8] & 0xC0, 0x80, "RFC 4122 variant bits");
        assert_eq!(second[6] >> 4, 4, "version nibble must be 4");
        assert_eq!(second[8] & 0xC0, 0x80, "RFC 4122 variant bits");
    }
}
