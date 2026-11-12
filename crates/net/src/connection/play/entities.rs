//! Entity packets of the M4.5 visibility stream.
//!
//! One [`StreamEntities`](crate::game::StreamEntities) event is turned into
//! the packets a viewer needs: the spawn sequence of a newly visible player,
//! the movement updates and the removals. The shapes and the order come from
//! the two-bot vanilla trace (С1–С4, `docs/ARCH-M4.md` §6); where the trace
//! does not establish a value it is carried as recorded and marked «не
//! установлено».
//!
//! SOURCE: two-bot vanilla trace of 26.3 (byte layout observed), not wiki.

use data::PacketIds;
use protocol::{
    AddEntity, AttributeEntry, EntityPositionSync, MoveEntityPos, MoveEntityPosRot, MoveEntityRot,
    MoveHeader, PacketKey, PlayerInfoEntry, PlayerInfoRemove, PlayerInfoUpdate, ProtocolState,
    RemoveEntities, RotateHead, SetEntityData, UpdateAttributes,
};
use tokio::io::AsyncWrite;

use crate::error::ConnectionError;
use crate::frame::write_tracked;
use crate::game::{
    byte_angle, pitch_byte, EntityMotion, EntityMove, EntityRemove, EntitySpawn, PlayerId,
    PLAYER_ENTITY_TYPE,
};

use super::super::PLAYER_INFO_GAME_MODE;

/// First metadata snapshot of a spawned player (`set_entity_data`, len 11).
///
/// The standing bot's value; the entry layout is «не установлено» and is
/// reproduced byte for byte.
// SOURCE: two-bot vanilla trace 26.3 (C1), payload `4d 09 03 41 a0 00 00 10 00 7f ff`.
const METADATA_SNAPSHOT: &[u8] = &[0x09, 0x03, 0x41, 0xA0, 0x00, 0x00, 0x10, 0x00, 0x7F];

/// Second metadata set of a spawned player (`set_entity_data`, len 14).
// SOURCE: two-bot vanilla trace 26.3 (C1), payload
// `4d 00 00 20 09 03 41 a0 00 00 10 00 7f ff`.
const METADATA_UPDATE: &[u8] = &[
    0x00, 0x00, 0x20, 0x09, 0x03, 0x41, 0xA0, 0x00, 0x00, 0x10, 0x00, 0x7F,
];

/// The three attributes the recorded join sends (`update_attributes`).
///
/// The wire order differs between vanilla runs (C1: 26, 13, 8; C2: 8, 26, 13),
/// so the parity comparison normalises it; here the C2 order is used.
// SOURCE: two-bot vanilla trace 26.3 (C1/C2), values 4.5, 0.1, 3.0.
fn standing_attributes(entity_id: PlayerId) -> UpdateAttributes {
    UpdateAttributes {
        entity_id,
        attributes: vec![
            AttributeEntry {
                key: 8,
                base: 4.5,
                modifiers: Vec::new(),
            },
            AttributeEntry {
                key: 26,
                // The trace carries 0.1 as an f32 widened to f64.
                base: f64::from(0.1_f32),
                modifiers: Vec::new(),
            },
            AttributeEntry {
                key: 13,
                base: 3.0,
                modifiers: Vec::new(),
            },
        ],
    }
}

/// Writes every packet of one entity-stream event.
pub(super) async fn write_entity_stream<W>(
    writer: &mut W,
    ids: &PacketIds,
    spawn: &[EntitySpawn],
    remove: &[EntityRemove],
    moves: &[EntityMove],
) -> Result<(), ConnectionError>
where
    W: AsyncWrite + Unpin,
{
    for entity in spawn {
        write_spawn(writer, ids, entity).await?;
    }
    for moved in moves {
        write_motion(writer, ids, moved).await?;
    }
    for entity in remove {
        write_remove(writer, ids, entity).await?;
    }
    Ok(())
}

/// Writes one packet with a runtime-resolved id.
async fn send<W>(
    writer: &mut W,
    ids: &PacketIds,
    key: PacketKey,
    payload: Vec<u8>,
) -> Result<(), ConnectionError>
where
    W: AsyncWrite + Unpin,
{
    write_tracked(writer, ProtocolState::Play, ids.id(key), &payload).await
}

/// The spawn sequence of a newly visible player, in the traced order:
/// `player_info_update`, `add_entity`, `set_entity_data`, the first
/// `move_entity_pos` (zero delta), `set_entity_data`, `update_attributes`.
async fn write_spawn<W>(
    writer: &mut W,
    ids: &PacketIds,
    entity: &EntitySpawn,
) -> Result<(), ConnectionError>
where
    W: AsyncWrite + Unpin,
{
    let uuid = entity.uuid;
    let info = PlayerInfoUpdate {
        players: vec![PlayerInfoEntry {
            uuid,
            name: entity.name.clone(),
            properties: Vec::new(),
            game_mode: PLAYER_INFO_GAME_MODE,
            listed: true,
            latency: 0,
        }],
    };
    send(writer, ids, PlayerInfoUpdate::KEY, info.encode_payload()?).await?;

    let add = AddEntity {
        entity_id: entity.id,
        uuid,
        entity_type: PLAYER_ENTITY_TYPE,
        x: entity.position.x,
        y: entity.position.y,
        z: entity.position.z,
        // The recorded angles are all zero and their order is «не установлено».
        pitch: 0,
        yaw: 0,
        head_yaw: 0,
        data: 0,
        tail: 0,
    };
    send(writer, ids, AddEntity::KEY, add.encode_payload()?).await?;

    let snapshot = SetEntityData {
        entity_id: entity.id,
        metadata: METADATA_SNAPSHOT.to_vec(),
    };
    send(writer, ids, SetEntityData::KEY, snapshot.encode_payload()?).await?;

    // The first movement packet after a spawn has a one-byte header (0x00) and
    // a zero delta; the header meaning is «не установлено».
    let first = MoveEntityPos {
        entity_id: entity.id,
        header: MoveHeader {
            flags: 0x00,
            kind: None,
        },
        dx: 0,
        dy: 0,
        dz: 0,
    };
    send(writer, ids, MoveEntityPos::KEY, first.encode_payload()?).await?;

    let update = SetEntityData {
        entity_id: entity.id,
        metadata: METADATA_UPDATE.to_vec(),
    };
    send(writer, ids, SetEntityData::KEY, update.encode_payload()?).await?;

    send(
        writer,
        ids,
        UpdateAttributes::KEY,
        standing_attributes(entity.id).encode_payload()?,
    )
    .await?;
    Ok(())
}

/// One movement update: relative position (optionally with rotation), a
/// rotation-only pair, or an absolute sync for a jump too large to encode.
async fn write_motion<W>(
    writer: &mut W,
    ids: &PacketIds,
    moved: &EntityMove,
) -> Result<(), ConnectionError>
where
    W: AsyncWrite + Unpin,
{
    let id = moved.id;
    let yaw = byte_angle(moved.position.yaw);
    let pitch = pitch_byte(moved.position.pitch);
    match moved.motion {
        EntityMotion::Position {
            dx,
            dy,
            dz,
            rotated,
        } => {
            // 0x03 when the position changed, with 0x02 as the second header
            // byte of the traced player; the meaning is «не установлено».
            let header = MoveHeader {
                flags: 0x03,
                kind: Some(0x02),
            };
            if rotated {
                let packet = MoveEntityPosRot {
                    entity_id: id,
                    header,
                    dx,
                    dy,
                    dz,
                    yaw,
                    pitch,
                };
                send(writer, ids, MoveEntityPosRot::KEY, packet.encode_payload()?).await?;
                let head = RotateHead {
                    entity_id: id,
                    head_yaw: yaw,
                };
                send(writer, ids, RotateHead::KEY, head.encode_payload()?).await?;
            } else {
                let packet = MoveEntityPos {
                    entity_id: id,
                    header,
                    dx,
                    dy,
                    dz,
                };
                send(writer, ids, MoveEntityPos::KEY, packet.encode_payload()?).await?;
            }
        }
        EntityMotion::Rotation => {
            let packet = MoveEntityRot {
                entity_id: id,
                // 0x00 on the first rotation packet, 0x01 afterwards; the
                // meaning is «не установлено». A rotation-only update can
                // only follow a spawn, so 0x01 is used.
                flags: 0x01,
                yaw,
                pitch,
            };
            send(writer, ids, MoveEntityRot::KEY, packet.encode_payload()?).await?;
            let head = RotateHead {
                entity_id: id,
                head_yaw: yaw,
            };
            send(writer, ids, RotateHead::KEY, head.encode_payload()?).await?;
        }
        EntityMotion::Sync => {
            let packet = EntityPositionSync {
                entity_id: id,
                // Header fields, the rotation field and the trailing byte are
                // «не установлено»; the recorded player values are used.
                teleport_id: 1,
                flags: 1,
                x: moved.position.x,
                y: moved.position.y,
                z: moved.position.z,
                rotation: 2,
                yaw: moved.position.yaw,
                pitch: moved.position.pitch,
                tail: false,
            };
            send(
                writer,
                ids,
                EntityPositionSync::KEY,
                packet.encode_payload()?,
            )
            .await?;
        }
    }
    Ok(())
}

/// The removal of a player: the tab-list entry only when it left the world,
/// then the entity itself.
async fn write_remove<W>(
    writer: &mut W,
    ids: &PacketIds,
    entity: &EntityRemove,
) -> Result<(), ConnectionError>
where
    W: AsyncWrite + Unpin,
{
    if entity.left_world {
        let info = PlayerInfoRemove {
            uuids: vec![entity.uuid],
        };
        send(writer, ids, PlayerInfoRemove::KEY, info.encode_payload()?).await?;
    }
    let remove = RemoveEntities {
        entity_ids: vec![entity.id],
    };
    send(writer, ids, RemoveEntities::KEY, remove.encode_payload()?).await?;
    Ok(())
}
