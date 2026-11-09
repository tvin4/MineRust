//! Connection handling: handshake parsing and the status, login,
//! configuration and play state machines.
//!
//! The states live in their own modules — [`login`] (login and
//! configuration) and [`play`] (the play loop) — this file reads the
//! handshake and routes the connection to the state it asks for.

mod login;
mod play;

use std::sync::Arc;
use std::time::Duration;

use data::{GameData, PacketIds};
use protocol::{
    decode_frame, Handshake, NextState, PingRequest, PongResponse, ProtocolState, StatusInfo,
    StatusRequest, StatusResponse,
};
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

use crate::error::ConnectionError;
use crate::frame::{read_tracked, write_tracked};
use crate::game::{self, GameHandle};
use crate::loader::Loader;

pub use play::WorldInfo;

/// What login and configuration learned about the client before play
/// starts: the name of Login Start and the view distance the client asked
/// for in configuration (`None` when it sent no `client_information`).
///
/// Grouped into one argument so the play loop stays readable — the name
/// drives the join sequence, the view distance caps the chunk radius.
#[derive(Debug)]
pub(super) struct Client {
    /// Player name of Login Start.
    pub(super) player_name: String,
    /// View distance of the last `client_information`, in chunks.
    pub(super) view_distance: Option<i32>,
}

/// Maximum time a connection may spend before reaching the play state
/// (handshake, status, login and configuration). Once in play there is no
/// global timeout: a joined client is kept alive by keep-alives instead.
// SOURCE: wiki, unverified (the timeout/keep-alive numbers are our own,
// matching the wiki's suggested intervals; only a live client proves them).
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a shutdown waits for the game thread to leave its tick loop
/// before the process is allowed to exit anyway.
const SHUTDOWN_JOIN_TIMEOUT: Duration = Duration::from_secs(2);

/// Accepts connections until Ctrl+C, then stops the game thread.
///
/// The status document, the loaded game data and the packet ids are shared
/// as immutable `Arc`s; there is no mutable shared state on this path.
/// `loader` is the chunk loader pool (`None` for a server without a world,
/// no chunks are sent) and `world` carries the join values of the save —
/// both read at startup on the server side.
pub async fn serve(
    listener: TcpListener,
    status: Arc<StatusInfo>,
    game_data: Arc<GameData>,
    packet_ids: Arc<PacketIds>,
    loader: Option<Loader>,
    world: WorldInfo,
) -> std::io::Result<()> {
    // One game thread owns the players of this server (ARCH-M4 §0). It runs
    // until this function returns; on shutdown it is asked to stop and
    // joined with a timeout, so a stuck tick cannot block the exit.
    let (game_thread, game) = game::GameHandle::spawn(game::GameConfig::default())?;
    let accepting = async move {
        loop {
            let (stream, peer) = listener.accept().await?;
            let status = Arc::clone(&status);
            let game_data = Arc::clone(&game_data);
            let packet_ids = Arc::clone(&packet_ids);
            let loader = loader.clone();
            let game = game.clone();
            tokio::spawn(async move {
                match handle_connection(stream, status, game_data, packet_ids, loader, world, game)
                    .await
                {
                    Ok(()) => debug!(%peer, "connection closed"),
                    Err(ConnectionError::TimedOut) => debug!(%peer, "connection timed out"),
                    Err(err) if err.is_client_disconnect() => {
                        debug!(%peer, %err, "client disconnected");
                    }
                    Err(err) => warn!(%peer, %err, "connection failed"),
                }
            });
        }
    };
    let result = tokio::select! {
        result = accepting => result,
        _ = shutdown_signal() => {
            info!("shutdown signal received");
            Ok(())
        }
    };
    if let Err(err) = game_thread.join_timeout(SHUTDOWN_JOIN_TIMEOUT) {
        warn!(%err, "the game thread did not stop in time");
    }
    result
}

/// Resolves when the process is asked to shut down.
///
/// Ctrl+C everywhere; on Windows also Ctrl+Break and a console-window close
/// (CTRL_CLOSE_EVENT), so closing the window stops the game thread instead
/// of being an unclean kill. Each signal is awaited by its own task, so one
/// failed handler never disturbs the others.
async fn shutdown_signal() {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);
    let ctrl_c_tx = tx.clone();
    tokio::spawn(async move {
        ctrl_c_or_pending().await;
        let _ = ctrl_c_tx.send(()).await;
    });
    #[cfg(windows)]
    watch_windows_signals(&tx);
    drop(tx);
    // Every sender is held by a parked task, so this only returns on a
    // signal (or when the runtime shuts down).
    let _ = rx.recv().await;
}

/// Spawns one waiter per Windows control event that means "stop".
#[cfg(windows)]
fn watch_windows_signals(tx: &tokio::sync::mpsc::Sender<()>) {
    if let Ok(mut signal) = tokio::signal::windows::ctrl_break() {
        let tx = tx.clone();
        tokio::spawn(async move {
            signal.recv().await;
            let _ = tx.send(()).await;
        });
    } else {
        warn!("could not listen for Ctrl+Break");
    }
    if let Ok(mut signal) = tokio::signal::windows::ctrl_close() {
        let tx = tx.clone();
        tokio::spawn(async move {
            signal.recv().await;
            let _ = tx.send(()).await;
        });
    } else {
        warn!("could not listen for the window-close signal");
    }
}

/// Ctrl+C; never resolves when the handler cannot be installed.
async fn ctrl_c_or_pending() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        warn!(%err, "could not listen for the shutdown signal");
        // Never resolve: without a signal there is nothing to react to.
        std::future::pending::<()>().await;
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
    loader: Option<Loader>,
    world: WorldInfo,
    game: GameHandle,
) -> Result<(), ConnectionError> {
    let ids = packet_ids.as_ref();
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut writer = write_half;

    let body = with_timeout(read_tracked(&mut reader, ProtocolState::Handshake)).await?;
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
            let client = with_timeout(login::login_state(
                &mut reader,
                &mut writer,
                &game_data,
                ids,
            ))
            .await?;
            play::play_state(
                reader,
                writer,
                play::Session {
                    game_data: &game_data,
                    status: &status,
                    client: &client,
                    ids,
                    world: play::World {
                        loader,
                        info: world,
                    },
                },
                game,
            )
            .await
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
        let body = read_tracked(reader, ProtocolState::Status).await?;
        let (packet_id, mut payload) = decode_frame(&body)?;
        match packet_id {
            // A client may also send a ping without a preceding request.
            id if id == ids.id(StatusRequest::KEY) => {
                StatusRequest::decode_payload(&mut payload)?;
                let response = StatusResponse {
                    json: status.to_json(),
                };
                write_tracked(
                    writer,
                    ProtocolState::Status,
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
                write_tracked(
                    writer,
                    ProtocolState::Status,
                    ids.id(PongResponse::KEY),
                    &pong.encode_payload(),
                )
                .await?;
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
