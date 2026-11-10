//! Scenario tests of the region grid, `Transfer` and the outgoing transcript
//! (M4.3 part B). Socket-free and in-process: the scheduler is driven directly.

use std::sync::Arc;
use std::time::Instant;

use protocol::Position;
use tokio::sync::mpsc;

use super::*;

/// A join registration at a block, with the player's movement slot and the
/// connection's side of the outbound queue.
fn join_at(
    id: PlayerId,
    spawn: (i32, i32, i32),
) -> (PlayerJoin, Arc<MoveSlot>, mpsc::Receiver<ServerMessage>) {
    let (outbound, receiver) = mpsc::channel(OUTBOUND_CAPACITY);
    let slot = Arc::new(MoveSlot::default());
    let join = PlayerJoin {
        id,
        name: format!("player-{id}"),
        view_distance: 10,
        spawn: Position {
            x: spawn.0,
            y: spawn.1,
            z: spawn.2,
        },
        connection: PlayerConnection {
            outbound,
            kick: Arc::new(KickFlag::default()),
            last_move: Arc::clone(&slot),
        },
    };
    (join, slot, receiver)
}

/// Payload of `move_player_pos`: three big-endian doubles and the flags byte.
fn pos_move(x: f64, y: f64, z: f64) -> MoveState {
    let mut payload = Vec::with_capacity(25);
    for value in [x, y, z] {
        payload.extend_from_slice(&value.to_be_bytes());
    }
    payload.push(0x01);
    MoveState {
        kind: MoveKind::Pos,
        payload,
    }
}

#[test]
fn a_region_visits_players_in_id_order_regardless_of_insertion_order() {
    // A `HashMap` would iterate in a per-process random order; the `BTreeMap`
    // of `Region` must make the moved ids strictly ascending whatever order
    // the players were inserted in (M4.3 B1.4).
    let grid = RegionGrid::new(2);
    let moved_ids = |order: [PlayerId; 3]| {
        let mut region = Region::new(SINGLE_REGION);
        let mut slots = Vec::new();
        for id in order {
            let (join, slot, _receiver) = join_at(id, (1, 64, 1));
            region.insert(join);
            region.get_mut(id).expect("inserted").confirm_teleport();
            slots.push((id, slot));
        }
        for (id, slot) in &slots {
            slot.store(pos_move(f64::from(*id), 64.0, 1.0));
        }
        let outcome = region.tick(0, Instant::now(), grid);
        outcome
            .moves
            .iter()
            .map(|moved| moved.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(moved_ids([3, 1, 2]), vec![1, 2, 3]);
    assert_eq!(moved_ids([2, 3, 1]), vec![1, 2, 3]);
}
