//! Shared helpers for the `server` integration tests: a real server on an
//! ephemeral port plus framed packet I/O bounded by [`IO_TIMEOUT`].
//!
//! Every test binary includes this module and uses a subset of it.

#![allow(dead_code)] // each test binary uses a different subset

use std::sync::Arc;
use std::time::Duration;

use protocol::{Handshake, NextState, StatusInfo};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Test configuration values; deliberately not the real game's.
pub const TEST_PROTOCOL_VERSION: i32 = 4242;
pub const TEST_VERSION_NAME: &str = "test-version";
pub const TEST_MAX_PLAYERS: i32 = 20;

/// Upper bound for any single network operation inside a test, so a
/// misbehaving server fails the test instead of hanging it.
pub const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// The status document the test server advertises.
pub fn test_status() -> StatusInfo {
    StatusInfo {
        version_name: TEST_VERSION_NAME.to_owned(),
        protocol_version: TEST_PROTOCOL_VERSION,
        max_players: TEST_MAX_PLAYERS,
        online_players: 0,
        motd: "Integration test MOTD".to_owned(),
    }
}

/// Starts the server with empty game data on an ephemeral port.
pub async fn start_server() -> std::net::SocketAddr {
    start_server_with_data(Arc::new(data::GameData::default())).await
}

/// Starts the server with explicit game data on an ephemeral port.
pub async fn start_server_with_data(game_data: Arc<data::GameData>) -> std::net::SocketAddr {
    start_server_with_ids(game_data, Arc::new(data::PacketIds::compiled())).await
}

/// Starts the server with explicit game data and explicit packet ids.
pub async fn start_server_with_ids(
    game_data: Arc<data::GameData>,
    packet_ids: Arc<data::PacketIds>,
) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(net::serve(
        listener,
        Arc::new(test_status()),
        game_data,
        packet_ids,
    ));
    addr
}

/// Sends one framed packet.
pub async fn send(stream: &mut TcpStream, packet_id: u32, payload: &[u8]) {
    let frame = protocol::encode_frame(packet_id, payload).expect("encodable");
    stream.write_all(&frame).await.expect("write");
    stream.flush().await.expect("flush");
}

/// Reads one framed packet and splits it into id and payload bytes.
///
/// Fails the test if the server does not answer within [`IO_TIMEOUT`].
pub async fn recv(stream: &mut TcpStream) -> (u32, Vec<u8>) {
    tokio::time::timeout(IO_TIMEOUT, recv_frame(stream))
        .await
        .expect("timed out waiting for a packet")
}

/// Reads one framed packet and splits it into id and payload bytes.
async fn recv_frame(stream: &mut TcpStream) -> (u32, Vec<u8>) {
    // Length prefix: at most 3 bytes per the protocol, read one at a time.
    let mut length = 0usize;
    let mut shift = 0;
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).await.expect("read length");
        length |= usize::from(byte[0] & 0x7F) << shift;
        if byte[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
        assert!(shift < 21, "length field too long");
    }

    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).await.expect("read body");
    let (id, consumed) = protocol::decode_varint(&body).expect("packet id");
    (id as u32, body[consumed..].to_vec())
}

/// Reads until the server closes the connection; returns the number of bytes
/// read before the close (must be 0 for a clean close).
pub async fn read_until_close(stream: &mut TcpStream) -> usize {
    tokio::time::timeout(IO_TIMEOUT, async {
        let mut tail = [0u8; 1];
        stream.read(&mut tail).await.expect("read")
    })
    .await
    .expect("timed out waiting for the server to close")
}

/// Performs the handshake for the status state.
pub async fn handshake(stream: &mut TcpStream) {
    let packet = Handshake {
        protocol_version: TEST_PROTOCOL_VERSION,
        server_address: "localhost".to_owned(),
        server_port: 25565,
        next_state: NextState::Status,
    };
    let payload = packet.encode_payload().expect("encodable");
    send(stream, Handshake::ID, &payload).await;
}
