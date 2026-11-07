//! Connection handling: handshake parsing and the status, login and
//! configuration state machines.

use std::sync::Arc;
use std::time::Duration;

use data::GameData;
use protocol::{
    decode_frame, offline_uuid, ClientInformation, FinishAck, FinishConfiguration, GameProfile,
    Handshake, LoginAcknowledged, LoginStart, LoginSuccess, NextState, PingRequest, PongResponse,
    RegistryData, RegistryEntry, RegistryTag, StatusInfo, StatusRequest, StatusResponse,
    TagRegistry, UpdateTags, MAX_STRING_CHARS,
};
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

use crate::error::ConnectionError;
use crate::frame::{read_frame, write_frame};

/// Maximum time a single connection may stay open.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Serverbound Plugin Message packet id in the configuration state. The
/// channel name is logged; the payload is ignored (M2 has no gameplay).
const PLUGIN_MESSAGE_ID: u32 = 0x02;

/// Accepts connections forever, spawning one task per connection.
///
/// The status document and the loaded game data are shared as immutable
/// `Arc`s; there is no mutable shared state on this path.
pub async fn serve(
    listener: TcpListener,
    status: Arc<StatusInfo>,
    game_data: Arc<GameData>,
) -> std::io::Result<()> {
    loop {
        let (stream, peer) = listener.accept().await?;
        let status = Arc::clone(&status);
        let game_data = Arc::clone(&game_data);
        tokio::spawn(async move {
            let result = tokio::time::timeout(
                CONNECTION_TIMEOUT,
                handle_connection(stream, status, game_data),
            )
            .await;
            match result {
                Ok(Ok(())) => debug!(%peer, "connection closed"),
                Ok(Err(err)) if err.is_client_disconnect() => {
                    debug!(%peer, %err, "client disconnected");
                }
                Ok(Err(err)) => warn!(%peer, %err, "connection failed"),
                Err(_) => debug!(%peer, "connection timed out"),
            }
        });
    }
}

/// Drives one connection from the handshake to the end of its state.
async fn handle_connection(
    stream: TcpStream,
    status: Arc<StatusInfo>,
    game_data: Arc<GameData>,
) -> Result<(), ConnectionError> {
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = write_half;

    let body = read_frame(&mut reader).await?;
    let (packet_id, mut payload) = decode_frame(&body)?;
    if packet_id != Handshake::ID {
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
        NextState::Status => status_state(&mut reader, &mut writer, &status).await,
        NextState::Login => login_state(&mut reader, &mut writer, &game_data).await,
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
            id if id == StatusRequest::ID => {
                StatusRequest::decode_payload(&mut payload)?;
                let response = StatusResponse {
                    json: status.to_json(),
                };
                write_frame(writer, StatusResponse::ID, &response.encode_payload()?).await?;
            }
            id if id == PingRequest::ID => {
                let ping = PingRequest::decode_payload(&mut payload)?;
                payload.ensure_empty()?;
                let pong = PongResponse {
                    payload: ping.payload,
                };
                write_frame(writer, PongResponse::ID, &pong.encode_payload()).await?;
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
/// the client confirms with Login Acknowledged to reach configuration.
async fn login_state<R, W>(
    reader: &mut R,
    writer: &mut W,
    game_data: &GameData,
) -> Result<(), ConnectionError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let body = read_frame(reader).await?;
    let (packet_id, mut payload) = decode_frame(&body)?;
    if packet_id != LoginStart::ID {
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
        session_id: uuid,
    };
    write_frame(writer, LoginSuccess::ID, &success.encode_payload()?).await?;

    let body = read_frame(reader).await?;
    let (packet_id, mut payload) = decode_frame(&body)?;
    if packet_id != LoginAcknowledged::ID {
        return Err(ConnectionError::UnexpectedPacket {
            id: packet_id,
            state: "login",
        });
    }
    LoginAcknowledged::decode_payload(&mut payload)?;
    payload.ensure_empty()?;
    debug!("login acknowledged, entering configuration");

    configuration_state(reader, writer, game_data).await
}

/// Serves the configuration state: sends every synchronized registry, the
/// tags and Finish Configuration, then waits for the client's Finish
/// Acknowledged before the connection would move on to play (milestone M2
/// step 5 stops here).
async fn configuration_state<R, W>(
    reader: &mut R,
    writer: &mut W,
    game_data: &GameData,
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
        write_frame(writer, RegistryData::ID, &packet.encode_payload()?).await?;
    }

    let tags = UpdateTags {
        registries: game_data
            .tags()
            .iter()
            .map(|(registry_id, tags)| TagRegistry {
                registry_id: registry_id.clone(),
                tags: tags
                    .iter()
                    .map(|(name, ids)| RegistryTag {
                        name: name.clone(),
                        entries: ids.clone(),
                    })
                    .collect(),
            })
            .collect(),
    };
    write_frame(writer, UpdateTags::ID, &tags.encode_payload()?).await?;
    write_frame(
        writer,
        FinishConfiguration::ID,
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
            id if id == ClientInformation::ID => {
                let info = ClientInformation::decode_payload(&mut payload)?;
                payload.ensure_empty()?;
                debug!(
                    locale = %info.locale,
                    view_distance = info.view_distance,
                    "client information received"
                );
            }
            id if id == PLUGIN_MESSAGE_ID => {
                // The channel name is worth logging; M2 ignores the payload.
                let channel = payload.read_string(MAX_STRING_CHARS)?;
                debug!(%channel, "plugin message received, ignored");
            }
            id if id == FinishAck::ID => {
                FinishAck::decode_payload(&mut payload)?;
                payload.ensure_empty()?;
                debug!("configuration acknowledged; play state arrives in M2 step 5");
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
