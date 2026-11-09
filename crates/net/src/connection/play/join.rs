//! The join sequence: the packets that place a freshly logged-in client
//! into the world.
//!
//! The block lives in its own file so [`super::play_state`] stays readable
//! (the project keeps files below ~500 lines) and so the order of these
//! packets is documented in one place. The order is:
//!
//! 1. Login (play) — dimension, view distance, hashed seed;
//! 2. Set Default Spawn Position — the save's spawn;
//! 3. Player Info Update — the player into the tab list;
//! 4. Set Center Chunk — the chunk cache centre, from the spawn;
//! 5. Game Event 13 — "start waiting for chunks" (closes the loading
//!    screen);
//! 6. Synchronize Player Position — the join teleport.
//!
//! SOURCE: wiki, unverified for this order and these field values; the live
//! session trace (`MINECRAFT_SESSION_TRACE`) is what will line them up
//! against the vanilla join recording packet by packet.

use data::{GameData, PacketIds};
use protocol::{
    GameEvent, LoginPlay, PlayerInfoEntry, PlayerInfoUpdate, ProtocolState, SetCenterChunk,
    SetDefaultSpawnPosition, StatusInfo, SynchronizePlayerPosition,
};
use tokio::io::AsyncWrite;
use tracing::{info, warn};

use crate::error::ConnectionError;
use crate::frame::write_tracked;

use super::{WorldInfo, DIMENSION_TYPE_REGISTRY, JOIN_TELEPORT_ID, OVERWORLD, SEA_LEVEL};

/// Sends the whole join sequence described in the module documentation.
///
/// Every packet is recorded by [`crate::trace`] as it goes out, so a live
/// session shows this exact order with names, ids, sizes and timing.
/// `entity_id` comes from the game thread's counter, so the first player
/// of a server keeps the id the join always announced.
// SOURCE: wiki, unverified (the join order and field values).
pub(super) async fn send_join<W>(
    writer: &mut W,
    game_data: &GameData,
    status: &StatusInfo,
    player_name: &str,
    entity_id: i32,
    ids: &PacketIds,
    info: &WorldInfo,
) -> Result<(), ConnectionError>
where
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
    // SOURCE: half-verified (the recording proves "not the registry"; our
    // world is the overworld alone for now).
    let dimension_names = vec![OVERWORLD.to_owned()];

    let login = LoginPlay {
        entity_id,
        hardcore: false,
        dimension_names,
        max_players: status.max_players,
        view_distance: info.view_distance,
        simulation_distance: info.simulation_distance,
        reduced_debug_info: false,
        enable_respawn_screen: true,
        do_limited_crafting: false,
        dimension_type,
        dimension_name: OVERWORLD.to_owned(),
        // Step 3.3e: `world_gen_settings.dat`'s seed, hashed by the
        // formula the parity recording verified (test `login_fields`);
        // without a world the default stays zero.
        hashed_seed: info.hashed_seed,
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
    write_tracked(
        writer,
        ProtocolState::Play,
        ids.id(LoginPlay::KEY),
        &login.encode_payload()?,
    )
    .await?;

    // Step 3.3e: the spawn of the save (`level.dat`) is where the player
    // joins — it also drives the chunk cache centre below, which vanilla
    // takes from the player's position.
    let spawn = SetDefaultSpawnPosition {
        dimension: OVERWORLD.to_owned(),
        position: info.spawn,
        yaw: 0.0,
        pitch: 0.0,
    };
    write_tracked(
        writer,
        ProtocolState::Play,
        ids.id(SetDefaultSpawnPosition::KEY),
        &spawn.encode_payload()?,
    )
    .await?;

    let uuid = protocol::offline_uuid(player_name);
    let player_info = PlayerInfoUpdate {
        players: vec![PlayerInfoEntry {
            uuid,
            name: player_name.to_owned(),
            properties: Vec::new(),
            game_mode: 0,
            listed: true,
            latency: 0,
        }],
    };
    write_tracked(
        writer,
        ProtocolState::Play,
        ids.id(PlayerInfoUpdate::KEY),
        &player_info.encode_payload()?,
    )
    .await?;

    // Centre of the cache: the position the player joined at (`spawn`),
    // expressed in chunks — vanilla keeps its view centred on the player.
    let center = SetCenterChunk {
        chunk_x: info.spawn.x >> 4,
        chunk_z: info.spawn.z >> 4,
    };
    write_tracked(
        writer,
        ProtocolState::Play,
        ids.id(SetCenterChunk::KEY),
        &center.encode_payload(),
    )
    .await?;
    info!(
        center_x = center.chunk_x,
        center_z = center.chunk_z,
        view_distance = info.view_distance,
        "chunk cache centre sent"
    );

    // Without event 13 the client stays on its loading screen.
    // SOURCE: wiki, unverified: the event id and the "close the loading
    // screen" effect were observed as documented, not compared to vanilla.
    let event = GameEvent {
        event: GameEvent::START_WAITING_FOR_CHUNKS,
        value: 0.0,
    };
    write_tracked(
        writer,
        ProtocolState::Play,
        ids.id(GameEvent::KEY),
        &event.encode_payload(),
    )
    .await?;

    let teleport = SynchronizePlayerPosition {
        teleport_id: JOIN_TELEPORT_ID,
        x: f64::from(info.spawn.x) + 0.5,
        y: f64::from(info.spawn.y),
        z: f64::from(info.spawn.z) + 0.5,
        velocity_x: 0.0,
        velocity_y: 0.0,
        velocity_z: 0.0,
        yaw: 0.0,
        pitch: 0.0,
        flags: 0, // fully absolute teleport
    };
    write_tracked(
        writer,
        ProtocolState::Play,
        ids.id(SynchronizePlayerPosition::KEY),
        &teleport.encode_payload(),
    )
    .await?;
    info!(player = %player_name, %entity_id, "player joined");
    Ok(())
}
