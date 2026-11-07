//! Integration test: a client performs the full server list ping against a
//! real server started on an ephemeral port, and a second test walks the
//! offline login all the way into the play state.
//!
//! All protocol values here are fictional test data, not real game values.

mod common;

use std::sync::Arc;

use protocol::{
    ClientInformation, ConfirmTeleportation, FinishAck, FinishConfiguration, GameEvent, Handshake,
    LoginAcknowledged, LoginPlay, LoginStart, LoginSuccess, NextState, PingRequest,
    PlayerInfoUpdate, PongResponse, SetCenterChunk, SetDefaultSpawnPosition, StatusRequest,
    StatusResponse, SynchronizePlayerPosition, UpdateTags,
};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use common::{
    handshake, read_until_close, recv, send, start_server, start_server_with_ids, TEST_MAX_PLAYERS,
    TEST_PROTOCOL_VERSION, TEST_VERSION_NAME,
};

/// The server must speak the packet ids of the loaded packets report, not
/// the compiled ones: the id of the status response is overridden in a
/// fixture report built from the compiled keys of the protocol crate.
#[tokio::test]
async fn status_response_comes_from_the_packets_report() {
    const OVERRIDE_ID: u32 = 0x11; // deliberately not the compiled default

    let report = data::fixtures::packets_report_json(&[(StatusResponse::KEY, OVERRIDE_ID)]);
    let ids = Arc::new(data::packets_from_json(&report).expect("fixture report parses"));
    let addr = start_server_with_ids(Arc::new(data::GameData::default()), ids).await;

    let mut stream = TcpStream::connect(addr).await.expect("connect");
    handshake(&mut stream).await;
    send(
        &mut stream,
        StatusRequest::ID,
        &StatusRequest.encode_payload(),
    )
    .await;

    let (packet_id, _) = recv(&mut stream).await;
    assert_eq!(packet_id, OVERRIDE_ID, "id must come from the report");
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
    assert_eq!(json["description"].as_str(), Some("Integration test MOTD"));

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
async fn offline_login_joins_the_play_state() {
    let addr = start_server().await;
    let mut stream = TcpStream::connect(addr).await.expect("connect");

    // Handshake into the login state.
    let packet = Handshake {
        protocol_version: TEST_PROTOCOL_VERSION,
        server_address: "localhost".to_owned(),
        server_port: 25565,
        next_state: NextState::Login,
    };
    send(
        &mut stream,
        Handshake::ID,
        &packet.encode_payload().expect("encodable"),
    )
    .await;

    // Login Start -> Login Success with the documented offline UUID.
    let name = "Steve";
    let start = LoginStart {
        name: name.to_owned(),
        uuid: [0; 16],
    };
    send(
        &mut stream,
        LoginStart::ID,
        &start.encode_payload().expect("encodable"),
    )
    .await;
    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, LoginSuccess::ID, "expected login success");
    let mut reader = protocol::PacketReader::new(&payload);
    let success = LoginSuccess::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!(success.profile.name, name);
    assert_eq!(success.profile.uuid, protocol::offline_uuid(name));
    // Vanilla draws a fresh random (version 4) session UUID per
    // connection; it never repeats the profile UUID.
    assert_ne!(success.session_id, success.profile.uuid);
    assert_eq!(success.session_id[6] >> 4, 4, "session uuid must be v4");
    assert!(success.profile.properties.is_empty());

    // Login Acknowledged -> configuration. With empty game data the server
    // skips Registry Data and sends Update Tags followed by Finish.
    send(
        &mut stream,
        LoginAcknowledged::ID,
        &LoginAcknowledged.encode_payload(),
    )
    .await;
    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, UpdateTags::ID, "expected update tags");
    let mut reader = protocol::PacketReader::new(&payload);
    let tags = UpdateTags::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert!(tags.registries.is_empty());

    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, FinishConfiguration::ID, "expected finish configuration");
    let mut reader = protocol::PacketReader::new(&payload);
    FinishConfiguration::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");

    // Client Information is accepted, then Finish Acknowledged switches
    // the connection to play.
    let info = ClientInformation {
        locale: "en_GB".to_owned(),
        view_distance: 8,
        chat_mode: 0,
        chat_colors: true,
        skin_parts: 0x7F,
        main_hand: 1,
        text_filtering: false,
        allow_server_listings: true,
        particle_status: 0,
    };
    send(
        &mut stream,
        ClientInformation::ID,
        &info.encode_payload().expect("encodable"),
    )
    .await;
    send(&mut stream, FinishAck::ID, &FinishAck.encode_payload()).await;

    // Join sequence: Login, spawn point, tab list entry, chunk center,
    // "start waiting for chunks" and the initial teleport. The test server
    // runs with empty game data, so the dimension type falls back to id 0.
    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, LoginPlay::ID, "expected login (play)");
    let mut reader = protocol::PacketReader::new(&payload);
    let login = LoginPlay::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!(login.entity_id, 1);
    assert!(!login.hardcore);
    assert_eq!(login.max_players, TEST_MAX_PLAYERS);
    assert_eq!(login.view_distance, 10);
    assert_eq!(login.simulation_distance, 10);
    assert_eq!(login.dimension_type, 0);
    assert_eq!(login.dimension_name, "minecraft:overworld");
    assert_eq!(login.game_mode, 0);
    assert_eq!(login.sea_level, 63);
    assert!(!login.online_mode);
    assert!(!login.enforces_secure_chat);
    assert!(login.death_location.is_none());

    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, SetDefaultSpawnPosition::ID, "expected spawn position");
    let mut reader = protocol::PacketReader::new(&payload);
    let spawn = SetDefaultSpawnPosition::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!(spawn.dimension, "minecraft:overworld");
    assert_eq!(spawn.position, protocol::Position { x: 8, y: 64, z: 8 });

    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, PlayerInfoUpdate::ID, "expected player info");
    let mut reader = protocol::PacketReader::new(&payload);
    let player_info = PlayerInfoUpdate::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!(player_info.players.len(), 1);
    assert_eq!(player_info.players[0].name, name);
    assert_eq!(player_info.players[0].uuid, protocol::offline_uuid(name));
    assert!(player_info.players[0].listed);

    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, SetCenterChunk::ID, "expected chunk center");
    let mut reader = protocol::PacketReader::new(&payload);
    let center = SetCenterChunk::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!((center.chunk_x, center.chunk_z), (0, 0));

    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, GameEvent::ID, "expected game event");
    let mut reader = protocol::PacketReader::new(&payload);
    let event = GameEvent::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!(event.event, 13, "must start waiting for chunks");

    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, SynchronizePlayerPosition::ID, "expected teleport");
    let mut reader = protocol::PacketReader::new(&payload);
    let teleport = SynchronizePlayerPosition::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!(teleport.teleport_id, 1);
    assert_eq!((teleport.x, teleport.y, teleport.z), (8.5, 64.0, 8.5));
    assert_eq!(teleport.flags, 0);

    // Serverbound play packets that carry no gameplay in M2 are accepted:
    // the teleport confirmation, updated settings and a plugin message.
    let confirm = ConfirmTeleportation {
        teleport_id: 1,
        x: 8.5,
        y: 64.0,
        z: 8.5,
        yaw: 0.0,
        pitch: 0.0,
    };
    send(
        &mut stream,
        ConfirmTeleportation::ID,
        &confirm.encode_payload(),
    )
    .await;
    send(
        &mut stream,
        0x0E,
        &info.encode_payload().expect("encodable"),
    )
    .await;
    let mut plugin = protocol::PacketWriter::new();
    plugin
        .write_string("minecraft:register", protocol::MAX_STRING_CHARS)
        .expect("channel");
    send(&mut stream, 0x16, plugin.as_bytes()).await;

    // Closing the client's write side ends the play state; the server then
    // drops the connection.
    stream.shutdown().await.expect("shutdown write");
    let read = read_until_close(&mut stream).await;
    assert_eq!(read, 0, "server should close the connection");
}
