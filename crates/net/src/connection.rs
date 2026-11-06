//! Connection handling: handshake parsing and the status state machine.

use std::sync::Arc;
use std::time::Duration;

use protocol::{
    decode_frame, Handshake, NextState, PingRequest, PongResponse, StatusInfo, StatusRequest,
    StatusResponse,
};
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, warn};

use crate::error::ConnectionError;
use crate::frame::{read_frame, write_frame};

/// Maximum time a single connection may stay open.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

/// Accepts connections forever, spawning one task per connection.
///
/// The status document is shared as an immutable `Arc`; there is no mutable
/// shared state on this path.
pub async fn serve(listener: TcpListener, status: Arc<StatusInfo>) -> std::io::Result<()> {
    loop {
        let (stream, peer) = listener.accept().await?;
        let status = Arc::clone(&status);
        tokio::spawn(async move {
            let result =
                tokio::time::timeout(CONNECTION_TIMEOUT, handle_connection(stream, status)).await;
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
        // Login and transfer are implemented in milestone M2; for now the
        // connection is closed without entering those states.
        NextState::Login | NextState::Transfer => {
            debug!(
                next_state = ?handshake.next_state,
                "state not implemented yet (M2), closing connection"
            );
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
