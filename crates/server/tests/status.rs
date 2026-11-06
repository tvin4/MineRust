//! Integration test: a client performs the full server list ping against a
//! real server started on an ephemeral port.
//!
//! All protocol values here are fictional test data, not real game values.

use std::sync::Arc;
use std::time::Duration;

use protocol::{
    Handshake, NextState, PingRequest, PongResponse, StatusInfo, StatusRequest, StatusResponse,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Test configuration values; deliberately not the real game's.
const TEST_PROTOCOL_VERSION: i32 = 4242;
const TEST_VERSION_NAME: &str = "test-version";
const TEST_MAX_PLAYERS: i32 = 20;

/// Upper bound for any single network operation inside a test, so a
/// misbehaving server fails the test instead of hanging it.
const IO_TIMEOUT: Duration = Duration::from_secs(5);

fn test_status() -> StatusInfo {
    StatusInfo {
        version_name: TEST_VERSION_NAME.to_owned(),
        protocol_version: TEST_PROTOCOL_VERSION,
        max_players: TEST_MAX_PLAYERS,
        online_players: 0,
        motd: "Integration test MOTD".to_owned(),
    }
}

/// Starts the server on an ephemeral port and returns its address.
async fn start_server() -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(net::serve(listener, Arc::new(test_status())));
    addr
}

/// Sends one framed packet.
async fn send(stream: &mut TcpStream, packet_id: u32, payload: &[u8]) {
    let frame = protocol::encode_frame(packet_id, payload).expect("encodable");
    stream.write_all(&frame).await.expect("write");
    stream.flush().await.expect("flush");
}

/// Reads one framed packet and splits it into id and payload bytes.
///
/// Fails the test if the server does not answer within [`IO_TIMEOUT`].
async fn recv(stream: &mut TcpStream) -> (u32, Vec<u8>) {
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
async fn read_until_close(stream: &mut TcpStream) -> usize {
    tokio::time::timeout(IO_TIMEOUT, async {
        let mut tail = [0u8; 1];
        stream.read(&mut tail).await.expect("read")
    })
    .await
    .expect("timed out waiting for the server to close")
}

/// Performs the handshake for the status state.
async fn handshake(stream: &mut TcpStream) {
    let packet = Handshake {
        protocol_version: TEST_PROTOCOL_VERSION,
        server_address: "localhost".to_owned(),
        server_port: 25565,
        next_state: NextState::Status,
    };
    let payload = packet.encode_payload().expect("encodable");
    send(stream, Handshake::ID, &payload).await;
}

#[tokio::test]
async fn server_list_ping_end_to_end() {
    let addr = start_server().await;
    let mut stream = TcpStream::connect(addr).await.expect("connect");

    handshake(&mut stream).await;

    // Status request -> JSON response.
    send(
        &mut stream,
        StatusRequest::ID,
        &StatusRequest.encode_payload(),
    )
    .await;
    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, StatusResponse::ID, "expected a status response");

    let mut reader = protocol::PacketReader::new(&payload);
    let response = StatusResponse::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");

    let json: serde_json::Value = serde_json::from_str(&response.json).expect("valid status json");
    assert_eq!(json["version"]["name"].as_str(), Some(TEST_VERSION_NAME));
    assert_eq!(
        json["version"]["protocol"].as_i64(),
        Some(i64::from(TEST_PROTOCOL_VERSION))
    );
    assert_eq!(
        json["players"]["max"].as_i64(),
        Some(i64::from(TEST_MAX_PLAYERS))
    );
    assert_eq!(json["players"]["online"].as_i64(), Some(0));
    assert_eq!(
        json["description"]["text"].as_str(),
        Some("Integration test MOTD")
    );

    // Ping -> pong with the same payload, then the server closes.
    let ping = PingRequest {
        payload: 0x0123_4567_89AB_CDEF,
    };
    send(&mut stream, PingRequest::ID, &ping.encode_payload()).await;
    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, PongResponse::ID, "expected a pong");

    let mut reader = protocol::PacketReader::new(&payload);
    let pong = PongResponse::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!(pong.payload, ping.payload);

    // The connection is closed after the pong.
    let read = read_until_close(&mut stream).await;
    assert_eq!(read, 0, "server should close the connection");
}

#[tokio::test]
async fn status_can_be_queried_repeatedly_on_one_connection() {
    let addr = start_server().await;
    let mut stream = TcpStream::connect(addr).await.expect("connect");

    handshake(&mut stream).await;
    for _ in 0..3 {
        send(
            &mut stream,
            StatusRequest::ID,
            &StatusRequest.encode_payload(),
        )
        .await;
        let (id, payload) = recv(&mut stream).await;
        assert_eq!(id, StatusResponse::ID);
        let mut reader = protocol::PacketReader::new(&payload);
        let response = StatusResponse::decode_payload(&mut reader).expect("decodable");
        assert!(!response.json.is_empty());
    }
}

#[tokio::test]
async fn login_state_is_closed_without_a_response() {
    let addr = start_server().await;
    let mut stream = TcpStream::connect(addr).await.expect("connect");

    // Login is milestone M2: the server must simply close the connection
    // instead of sending anything.
    let packet = Handshake {
        protocol_version: TEST_PROTOCOL_VERSION,
        server_address: "localhost".to_owned(),
        server_port: 25565,
        next_state: NextState::Login,
    };
    let payload = packet.encode_payload().expect("encodable");
    send(&mut stream, Handshake::ID, &payload).await;

    let read = read_until_close(&mut stream).await;
    assert_eq!(read, 0, "server should close the connection");
}
