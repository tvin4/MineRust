//! Integration test: the full offline join against a real server, with game
//! data loaded from an in-memory fixture jar — handshake, login,
//! configuration (registry data, tags, finish) and the play join sequence.
//!
//! Every registry, entry and tag below is fictional test data defined in
//! this file, not real game data.

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use protocol::{
    ClientInformation, ConfirmTeleportation, FinishAck, FinishConfiguration, GameEvent, Handshake,
    LoginAcknowledged, LoginPlay, LoginStart, LoginSuccess, NextState, PacketReader,
    PlayerInfoUpdate, RegistryData, SetCenterChunk, SetDefaultSpawnPosition,
    SynchronizePlayerPosition, UpdateTags,
};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use common::{
    read_until_close, recv, send, start_server_with_data, TEST_MAX_PLAYERS, TEST_PROTOCOL_VERSION,
};

/// Upper bound on registry packets per configuration, so a server bug fails
/// the test instead of spinning until the keep-alive era.
const MAX_REGISTRY_PACKETS: usize = 64;

/// Fictional game data: every synchronized registry gets one placeholder
/// entry, the dimension type registry gets three entries (so the numeric id
/// of the overworld is not 0), and one tag resolves to a known id.
fn fixture_game_data() -> Arc<data::GameData> {
    Arc::new(
        data::load_from_zip(data::fixtures::jar_with(&[
            (
                "data/minecraft/dimension_type/aaa.json",
                r#"{"piglin_safe": false}"#,
            ),
            (
                "data/minecraft/dimension_type/overworld.json",
                r#"{"has_skylight": true}"#,
            ),
            ("data/minecraft/dimension_type/the_end.json", "{}"),
            (
                "data/minecraft/damage_type/fixture.json",
                r#"{"message_id": "fixture"}"#,
            ),
            (
                "data/minecraft/tags/damage_type/is_fire.json",
                r#"{"values": ["minecraft:fixture"]}"#,
            ),
        ]))
        .expect("fixture jar loads"),
    )
}

#[tokio::test]
async fn full_join_handshake_reaches_play_with_registry_data() {
    let game_data = fixture_game_data();
    let addr = start_server_with_data(Arc::clone(&game_data)).await;
    let mut stream = TcpStream::connect(addr).await.expect("connect");

    // Handshake into the login state, then Login Start -> Login Success.
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
    let mut reader = PacketReader::new(&payload);
    let success = LoginSuccess::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!(success.profile.uuid, protocol::offline_uuid(name));

    // Login Acknowledged -> configuration: one Registry Data packet per
    // synchronized registry, then Update Tags.
    send(
        &mut stream,
        LoginAcknowledged::ID,
        &LoginAcknowledged.encode_payload(),
    )
    .await;

    let mut registry_ids = BTreeSet::new();
    let mut packets = 0usize;
    loop {
        let (id, payload) = recv(&mut stream).await;
        if id == UpdateTags::ID {
            let mut reader = PacketReader::new(&payload);
            let tags = UpdateTags::decode_payload(&mut reader).expect("decodable");
            reader.ensure_empty().expect("no trailing bytes");

            let damage = tags
                .registries
                .iter()
                .find(|registry| registry.registry_id == "minecraft:damage_type")
                .expect("damage type tags");
            let is_fire = damage
                .tags
                .iter()
                .find(|tag| tag.name == "minecraft:is_fire")
                .expect("is_fire tag");
            // `minecraft:fixture` is the only damage type entry: id 0.
            assert_eq!(is_fire.entries, [0]);
            break;
        }
        assert_eq!(
            id,
            RegistryData::ID,
            "expected registry data before update tags"
        );
        packets += 1;
        assert!(packets <= MAX_REGISTRY_PACKETS, "too many registry packets");

        let mut reader = PacketReader::new(&payload);
        let packet = RegistryData::decode_payload(&mut reader).expect("decodable");
        reader.ensure_empty().expect("no trailing bytes");

        // The packet must mirror the loaded data: same entries in the same
        // order, because the position in the packet is the numeric id.
        let registry = game_data
            .registry(&packet.registry_id)
            .expect("registry was loaded from the fixture jar");
        let expected: Vec<&str> = registry
            .entries()
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        let got: Vec<&str> = packet
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(got, expected, "entries of {}", packet.registry_id);
        assert!(
            packet.entries.iter().all(|entry| entry.data.is_some()),
            "fixture entries must carry their data"
        );
        registry_ids.insert(packet.registry_id);
    }
    let expected_ids: BTreeSet<String> = data::SYNCHRONIZED_REGISTRIES
        .iter()
        .map(|dir| format!("minecraft:{dir}"))
        .collect();
    assert_eq!(registry_ids, expected_ids, "every registry exactly once");

    // Finish Configuration ends the registry exchange.
    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, FinishConfiguration::ID, "expected finish configuration");
    let mut reader = PacketReader::new(&payload);
    FinishConfiguration::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");

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

    // Join sequence. The dimension type id comes from the registry: the
    // fixture sorts `minecraft:aaa` first, so the overworld is id 1.
    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, LoginPlay::ID, "expected login (play)");
    let mut reader = PacketReader::new(&payload);
    let login = LoginPlay::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!(
        login.dimension_names,
        ["minecraft:aaa", "minecraft:overworld", "minecraft:the_end"]
    );
    assert_eq!(login.dimension_type, 1, "overworld id from the registry");
    assert_eq!(login.dimension_name, "minecraft:overworld");
    assert_eq!(login.entity_id, 1);
    assert_eq!(login.max_players, TEST_MAX_PLAYERS);
    assert_eq!(login.view_distance, 10);
    assert_eq!(login.sea_level, 63);
    assert!(!login.online_mode);

    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, SetDefaultSpawnPosition::ID, "expected spawn position");
    let mut reader = PacketReader::new(&payload);
    let spawn = SetDefaultSpawnPosition::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!(spawn.position, protocol::Position { x: 8, y: 64, z: 8 });

    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, PlayerInfoUpdate::ID, "expected player info");
    let mut reader = PacketReader::new(&payload);
    let player_info = PlayerInfoUpdate::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!(player_info.players.len(), 1);
    assert_eq!(player_info.players[0].name, name);
    assert_eq!(player_info.players[0].uuid, protocol::offline_uuid(name));

    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, SetCenterChunk::ID, "expected chunk center");
    let mut reader = PacketReader::new(&payload);
    let center = SetCenterChunk::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!((center.chunk_x, center.chunk_z), (0, 0));

    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, GameEvent::ID, "expected game event");
    let mut reader = PacketReader::new(&payload);
    let event = GameEvent::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!(event.event, 13, "must start waiting for chunks");

    let (id, payload) = recv(&mut stream).await;
    assert_eq!(id, SynchronizePlayerPosition::ID, "expected teleport");
    let mut reader = PacketReader::new(&payload);
    let teleport = SynchronizePlayerPosition::decode_payload(&mut reader).expect("decodable");
    reader.ensure_empty().expect("no trailing bytes");
    assert_eq!(teleport.teleport_id, 1);
    assert_eq!((teleport.x, teleport.y, teleport.z), (8.5, 64.0, 8.5));
    assert_eq!(teleport.flags, 0);

    // Confirm the teleport, then close: the server must drop the socket.
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
    stream.shutdown().await.expect("shutdown write");
    let read = read_until_close(&mut stream).await;
    assert_eq!(read, 0, "server should close the connection");
}
