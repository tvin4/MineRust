//! Packet building for the M4.5 entity-visibility stream.
//!
//! A [`StreamEntities`](crate::game::StreamEntities) event is turned into the
//! packets a viewer needs: the spawn sequence of a newly visible player, the
//! movement updates and the removals. The shapes and the order come from the
//! two-bot vanilla trace (С1–С4, `docs/ARCH-M4.md` §6); where the trace does
//! not establish a value it is carried as recorded and marked «не
//! установлено».
//!
//! The module is the single place the packets are built: the connection
//! writes them and the Шаг 3.5 benchmark times the build alone, so the two
//! never drift. Every function pushes `(key, payload)` pairs, in send order,
//! into a caller buffer — the caller reuses it, so nothing is allocated per
//! event beyond the payloads themselves.
//!
//! SOURCE: two-bot vanilla trace of 26.3 (byte layout observed), not wiki.

use protocol::{
    AddEntity, AttributeEntry, CodecError, EntityPositionSync, MoveEntityPos, MoveEntityPosRot,
    MoveEntityRot, MoveHeader, PacketKey, PlayerInfoEntry, PlayerInfoRemove, PlayerInfoUpdate,
    RemoveEntities, RotateHead, SetEntityData, UpdateAttributes,
};

use crate::game::{
    byte_angle, pitch_byte, EntityMotion, EntityMove, EntityRemove, EntitySpawn, PlayerId,
    PLAYER_ENTITY_TYPE,
};

/// First metadata snapshot of a spawned player (`set_entity_data`, len 11).
///
/// The standing bot's value; the entry layout is «не установлено» and is
/// reproduced byte for byte. Identical in the survival (C5) and creative
/// (C6) traces.
// SOURCE: two-bot vanilla trace 26.3 (C1/C5/C6), payload `4d 09 03 41 a0 00 00 10 00 7f ff`.
const METADATA_SNAPSHOT: &[u8] = &[0x09, 0x03, 0x41, 0xA0, 0x00, 0x00, 0x10, 0x00, 0x7F];

/// Second metadata set of a spawned spectator (`set_entity_data`, len 14):
/// the record at index 0 (entity flags `0x20`) precedes the standing record.
///
/// Only the spectator trace C1 carries it; survival (C5) and creative (C6)
/// repeat [`METADATA_SNAPSHOT`] byte for byte. The meaning of the `0x20`
/// flag record is «не установлено»; it is reproduced for game mode 3 only.
/// Mode 2 (adventure) is untraced: it takes the C5/C6 path («не установлено»).
// SOURCE: two-bot vanilla trace 26.3 (C1), payload
// `4d 00 00 20 09 03 41 a0 00 00 10 00 7f ff`.
const METADATA_UPDATE: &[u8] = &[
    0x00, 0x00, 0x20, 0x09, 0x03, 0x41, 0xA0, 0x00, 0x00, 0x10, 0x00, 0x7F,
];

/// The second metadata set of a spawned player, selected by game mode:
/// the spectator record for mode 3, the plain snapshot for the rest.
fn metadata_second(game_mode: i32) -> &'static [u8] {
    if game_mode == 3 {
        METADATA_UPDATE
    } else {
        METADATA_SNAPSHOT
    }
}

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

/// Pushes the spawn sequence of a newly visible player, in the traced order:
/// `player_info_update`, `add_entity`, `set_entity_data`, the first
/// `move_entity_pos` (zero delta), `set_entity_data`, `update_attributes`.
pub fn push_spawn(
    entity: &EntitySpawn,
    out: &mut Vec<(PacketKey, Vec<u8>)>,
) -> Result<(), CodecError> {
    let uuid = entity.uuid;
    let info = PlayerInfoUpdate {
        players: vec![PlayerInfoEntry {
            uuid,
            name: entity.name.clone(),
            properties: Vec::new(),
            game_mode: entity.game_mode,
            listed: true,
            latency: 0,
        }],
    };
    out.push((PlayerInfoUpdate::KEY, info.encode_payload()?));

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
    out.push((AddEntity::KEY, add.encode_payload()?));

    let snapshot = SetEntityData {
        entity_id: entity.id,
        metadata: METADATA_SNAPSHOT.to_vec(),
    };
    out.push((SetEntityData::KEY, snapshot.encode_payload()?));

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
    out.push((MoveEntityPos::KEY, first.encode_payload()?));

    let update = SetEntityData {
        entity_id: entity.id,
        metadata: metadata_second(entity.game_mode).to_vec(),
    };
    out.push((SetEntityData::KEY, update.encode_payload()?));

    out.push((
        UpdateAttributes::KEY,
        standing_attributes(entity.id).encode_payload()?,
    ));
    Ok(())
}

/// Pushes one movement update: relative position (optionally with rotation),
/// a rotation-only pair, or an absolute sync for a jump too large to encode.
pub fn push_motion(
    moved: &EntityMove,
    out: &mut Vec<(PacketKey, Vec<u8>)>,
) -> Result<(), CodecError> {
    let id = moved.id;
    let game_mode = moved.game_mode;
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
                out.push((MoveEntityPosRot::KEY, packet.encode_payload()?));
                let head = RotateHead {
                    entity_id: id,
                    head_yaw: yaw,
                };
                out.push((RotateHead::KEY, head.encode_payload()?));
            } else {
                let packet = MoveEntityPos {
                    entity_id: id,
                    header,
                    dx,
                    dy,
                    dz,
                };
                out.push((MoveEntityPos::KEY, packet.encode_payload()?));
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
            out.push((MoveEntityRot::KEY, packet.encode_payload()?));
            let head = RotateHead {
                entity_id: id,
                head_yaw: yaw,
            };
            out.push((RotateHead::KEY, head.encode_payload()?));
        }
        EntityMotion::ZeroDelta => {
            // The periodic no-change packet: header flags `0x02` in every
            // spectator trace (C1/C2/C4) and `0x03` in the non-spectator ones
            // (C5 survival, C6 creative), the second byte `0x02` in both.
            // The meaning is «не установлено»; the mode is the only variable
            // that correlates with the difference.
            // SOURCE: two-bot vanilla traces 26.3 (C1 `02 02`, C5/C6 `03 02`).
            let flags = if game_mode == 3 { 0x02 } else { 0x03 };
            let packet = MoveEntityPos {
                entity_id: id,
                header: MoveHeader {
                    flags,
                    kind: Some(0x02),
                },
                dx: 0,
                dy: 0,
                dz: 0,
            };
            out.push((MoveEntityPos::KEY, packet.encode_payload()?));
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
            out.push((EntityPositionSync::KEY, packet.encode_payload()?));
        }
    }
    Ok(())
}

/// Pushes the removal of a player: the entity first, then the tab-list entry
/// when it left the world — the C1 order (`remove_entities` seq 16554,
/// `player_info_remove` seq 16556).
// SOURCE: two-bot vanilla trace 26.3 (C1).
pub fn push_remove(
    entity: &EntityRemove,
    out: &mut Vec<(PacketKey, Vec<u8>)>,
) -> Result<(), CodecError> {
    let remove = RemoveEntities {
        entity_ids: vec![entity.id],
    };
    out.push((RemoveEntities::KEY, remove.encode_payload()?));
    if entity.left_world {
        let info = PlayerInfoRemove {
            uuids: vec![entity.uuid],
        };
        out.push((PlayerInfoRemove::KEY, info.encode_payload()?));
    }
    Ok(())
}

/// Pushes every packet of one entity-stream event, in send order.
pub fn push_entity_stream(
    spawn: &[EntitySpawn],
    remove: &[EntityRemove],
    moves: &[EntityMove],
    out: &mut Vec<(PacketKey, Vec<u8>)>,
) -> Result<(), CodecError> {
    for entity in spawn {
        push_spawn(entity, out)?;
    }
    for moved in moves {
        push_motion(moved, out)?;
    }
    for entity in remove {
        push_remove(entity, out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{relative_delta, PlayerPos};
    use protocol::PacketReader;

    /// The world exit sends `remove_entities` first and the tab-list
    /// `player_info_remove` after it — the order recorded in C1
    /// (`remove_entities` seq 16554, `player_info_remove` seq 16556).
    // SOURCE: two-bot vanilla trace 26.3 (C1).
    #[test]
    fn world_exit_removes_the_entity_before_the_tab_entry() {
        let remove = EntityRemove {
            id: 77,
            uuid: [0xAB; 16],
            left_world: true,
        };
        let mut out = Vec::new();
        push_remove(&remove, &mut out).expect("encodable");
        let keys: Vec<PacketKey> = out.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys, vec![RemoveEntities::KEY, PlayerInfoRemove::KEY]);
    }

    /// Leaving the view radius (not the world) drops only the entity and
    /// leaves the tab-list entry.
    #[test]
    fn view_exit_removes_only_the_entity() {
        let remove = EntityRemove {
            id: 5,
            uuid: [0; 16],
            left_world: false,
        };
        let mut out = Vec::new();
        push_remove(&remove, &mut out).expect("encodable");
        let keys: Vec<PacketKey> = out.iter().map(|(key, _)| *key).collect();
        assert_eq!(keys, vec![RemoveEntities::KEY]);
    }

    /// The motion kinds map to the traced packets: a relative position to
    /// `move_entity_pos`, a position + rotation to `move_entity_pos_rot` +
    /// `rotate_head`, a rotation-only change to `move_entity_rot` +
    /// `rotate_head`, and a jump too large for the relative packets to
    /// `entity_position_sync`.
    #[test]
    fn motion_encodes_to_the_traced_packets() {
        let position = PlayerPos {
            x: 10.0,
            y: 64.0,
            z: -3.0,
            yaw: 90.0,
            pitch: -20.0,
            flags: 0,
        };
        let names = |motion| -> Vec<&'static str> {
            let moved = EntityMove {
                id: 7,
                position,
                motion,
                game_mode: 0,
            };
            let mut out = Vec::new();
            push_motion(&moved, &mut out).expect("encodable");
            out.iter().map(|(key, _)| key.name).collect()
        };
        let step = relative_delta(0.2).expect("0.2 blocks fit");
        assert_eq!(
            names(EntityMotion::Position {
                dx: step,
                dy: 0,
                dz: 0,
                rotated: false,
            }),
            vec!["minecraft:move_entity_pos"]
        );
        assert_eq!(
            names(EntityMotion::Position {
                dx: step,
                dy: 0,
                dz: 0,
                rotated: true,
            }),
            vec!["minecraft:move_entity_pos_rot", "minecraft:rotate_head"]
        );
        assert_eq!(
            names(EntityMotion::Rotation),
            vec!["minecraft:move_entity_rot", "minecraft:rotate_head"]
        );
        assert_eq!(
            names(EntityMotion::ZeroDelta),
            vec!["minecraft:move_entity_pos"]
        );
        assert_eq!(
            names(EntityMotion::Sync),
            vec!["minecraft:entity_position_sync"]
        );
    }

    /// The periodic no-change packet is `move_entity_pos` with a zero delta:
    /// header `02 02` in the spectator traces (C1/C2/C4) and `03 02` in the
    /// non-spectator ones (C5 survival, C6 creative), Шаг 6.2 + Шаг 7.3.
    // SOURCE: two-bot vanilla traces 26.3 (C1 `02 02`, C5/C6 `03 02`).
    #[test]
    fn zero_delta_encodes_the_traced_periodic_packet() {
        let moved = EntityMove {
            id: 0x4d,
            position: PlayerPos {
                x: 0.0,
                y: 0.0,
                z: 0.0,
                yaw: 0.0,
                pitch: 0.0,
                flags: 0,
            },
            motion: EntityMotion::ZeroDelta,
            game_mode: 3,
        };
        let mut out = Vec::new();
        push_motion(&moved, &mut out).expect("encodable");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, MoveEntityPos::KEY);
        assert_eq!(out[0].1, vec![0x4d, 0x02, 0x02, 0, 0, 0, 0, 0, 0]);
    }

    /// `teleport_entity` is out of M4.5 scope: neither a spawn nor any motion
    /// kind may emit it.
    #[test]
    fn the_stream_never_uses_teleport_entity() {
        let position = PlayerPos {
            x: 1.0,
            y: 64.0,
            z: 2.0,
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
        };
        let spawn = EntitySpawn {
            id: 3,
            name: "Bot".to_owned(),
            uuid: [0x11; 16],
            position,
            game_mode: 0,
        };
        let mut out = Vec::new();
        push_spawn(&spawn, &mut out).expect("encodable");
        for motion in [
            EntityMotion::Position {
                dx: 1,
                dy: 0,
                dz: 0,
                rotated: false,
            },
            EntityMotion::Position {
                dx: 1,
                dy: 0,
                dz: 0,
                rotated: true,
            },
            EntityMotion::Rotation,
            EntityMotion::ZeroDelta,
            EntityMotion::Sync,
        ] {
            push_motion(
                &EntityMove {
                    id: 3,
                    position,
                    motion,
                    game_mode: 0,
                },
                &mut out,
            )
            .expect("encodable");
        }
        for (key, _) in &out {
            assert_ne!(
                key.name, "minecraft:teleport_entity",
                "teleport_entity is not part of the M4.5 stream"
            );
        }
    }

    /// The periodic zero-delta header is game-mode dependent (Шаг 7.3):
    /// spectator keeps `02 02` (C1), every other mode sends `03 02` (C5/C6).
    // SOURCE: two-bot vanilla traces 26.3 (C1 vs C5/C6).
    #[test]
    fn the_zero_delta_header_depends_on_the_game_mode() {
        let header = |game_mode: i32| -> Vec<u8> {
            let moved = EntityMove {
                id: 1,
                position: PlayerPos {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    yaw: 0.0,
                    pitch: 0.0,
                    flags: 0,
                },
                motion: EntityMotion::ZeroDelta,
                game_mode,
            };
            let mut out = Vec::new();
            push_motion(&moved, &mut out).expect("encodable");
            out[0].1.clone()
        };
        assert_eq!(header(3), vec![0x01, 0x02, 0x02, 0, 0, 0, 0, 0, 0]);
        assert_eq!(header(0), vec![0x01, 0x03, 0x02, 0, 0, 0, 0, 0, 0]);
        assert_eq!(header(1), vec![0x01, 0x03, 0x02, 0, 0, 0, 0, 0, 0]);
        assert_eq!(header(2), vec![0x01, 0x03, 0x02, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn the_second_metadata_set_depends_on_the_game_mode() {
        // M4.5 Шаг 7.3: spectator (3) takes the C1 record with the leading
        // entity-flags entry; survival (0), creative (1) and adventure (2)
        // repeat the snapshot byte for byte (C5/C6; 2 is untraced).
        assert_eq!(metadata_second(3), METADATA_UPDATE, "spectator C1 bytes");
        assert_eq!(metadata_second(0), METADATA_SNAPSHOT, "survival C5 bytes");
        assert_eq!(metadata_second(1), METADATA_SNAPSHOT, "creative C6 bytes");
        assert_eq!(
            metadata_second(2),
            METADATA_SNAPSHOT,
            "adventure is untraced"
        );
    }

    #[test]
    fn the_tab_entry_carries_the_spawned_players_game_mode() {
        // M4.5 Шаг 7.3: the game mode of a tab-list entry is the spawned
        // player's own, not a constant copied from the spectator trace.
        let mut out = Vec::new();
        let spawn = EntitySpawn {
            id: 3,
            name: "Bot".to_owned(),
            uuid: [0x11; 16],
            position: PlayerPos {
                x: 1.0,
                y: 64.0,
                z: 2.0,
                yaw: 0.0,
                pitch: 0.0,
                flags: 0,
            },
            game_mode: 1,
        };
        push_spawn(&spawn, &mut out).expect("encodable");
        let mut reader = PacketReader::new(&out[0].1);
        let info = PlayerInfoUpdate::decode_payload(&mut reader).expect("decodable");
        assert_eq!(
            info.players[0].game_mode, 1,
            "creative from the spawn record"
        );
    }
}
