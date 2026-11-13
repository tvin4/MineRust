//! Tests of the player state step 8 adds: input flags (sneak/sprint), swing
//! events and the head yaw, all socket-free (the slot is driven directly).

use std::sync::Arc;

use protocol::Position;

use super::messages::{MoveKind, MoveSlot, MoveState, PlayerConnection, PlayerJoin};
use super::{KickFlag, MoveUpdate, Player, Region, SINGLE_REGION};

/// A player with a fresh slot, registered in a region and teleported acked.
fn player(slot: &Arc<MoveSlot>) -> Player {
    let (outbound, _receiver) = tokio::sync::mpsc::channel(4);
    let join = PlayerJoin {
        id: 7,
        name: "state-test".to_owned(),
        view_distance: 10,
        spawn: Position::default(),
        game_mode: 0,
        connection: PlayerConnection {
            outbound,
            kick: Arc::new(KickFlag::default()),
            last_move: Arc::clone(slot),
        },
    };
    let mut player = Player::new(join);
    player.confirm_teleport();
    player
}

/// The `player_input` flags byte is the source of truth for sneak and
/// sprint: the two bits land in the player state.
#[test]
fn the_input_flags_set_sneak_and_sprint() {
    let slot = Arc::new(MoveSlot::default());
    let mut player = player(&slot);
    assert!(!player.sneak());
    assert!(!player.sprint());

    player.apply_input(protocol::PlayerInput::SNEAK);
    assert!(player.sneak());
    assert!(!player.sprint());

    player.apply_input(protocol::PlayerInput::SPRINT);
    assert!(!player.sneak());
    assert!(player.sprint());

    player.apply_input(protocol::PlayerInput::SNEAK | protocol::PlayerInput::SPRINT);
    assert!(player.sneak());
    assert!(player.sprint());

    player.apply_input(0);
    assert!(!player.sneak());
    assert!(!player.sprint());
}

/// The slot keeps the newest input byte and the swing count independently of
/// movement: a tick without movement still applies them.
#[test]
fn the_slot_stores_input_and_swings_beside_movement() {
    let slot = Arc::new(MoveSlot::default());
    slot.store_input(protocol::PlayerInput::SNEAK);
    slot.store_swing();
    slot.store_swing();
    // A newer input byte wins, the swings accumulate.
    slot.store_input(protocol::PlayerInput::SPRINT);

    assert_eq!(slot.take_input(), Some(protocol::PlayerInput::SPRINT));
    assert_eq!(
        slot.take_input(),
        None,
        "the input entry is empty after take"
    );
    assert_eq!(slot.take_swings(), 2);
    assert_eq!(slot.take_swings(), 0, "the swing count is reset");
    assert_eq!(slot.take(), None, "movement is untouched by input/swings");
}

/// A movement packet does not swallow the pending input or swings (the
/// three categories are separate entries of the same slot).
#[test]
fn movement_does_not_swallow_the_other_categories() {
    let slot = Arc::new(MoveSlot::default());
    slot.store(MoveState {
        kind: MoveKind::Pos,
        payload: Vec::new(),
    });
    slot.store_input(protocol::PlayerInput::SPRINT);
    slot.store_swing();

    assert!(slot.take().is_some(), "the movement is still there");
    assert_eq!(slot.take_input(), Some(protocol::PlayerInput::SPRINT));
    assert_eq!(slot.take_swings(), 1);
}

/// The head yaw follows the body yaw: no separate head source is
/// established, so a rotation movement updates both together.
#[test]
fn the_head_yaw_follows_the_body_yaw() {
    let slot = Arc::new(MoveSlot::default());
    let mut player = player(&slot);
    let update = MoveUpdate {
        x: None,
        y: None,
        z: None,
        yaw: Some(30.0),
        pitch: Some(-10.0),
        flags: 0,
    };
    player.apply_move(&update);
    let position = player.position();
    assert_eq!(position.yaw, 30.0);
    assert_eq!(position.head_yaw, 30.0, "the head follows the body");
    assert_eq!(position.pitch, -10.0);
}

/// The tick applies input and swings even when no movement packet arrived
/// (the region loop drains every category of the slot).
#[test]
fn the_tick_applies_input_without_movement() {
    let mut region = Region::new(SINGLE_REGION);
    let slot = Arc::new(MoveSlot::default());
    let (outbound, _receiver) = tokio::sync::mpsc::channel(4);
    let join = PlayerJoin {
        id: 3,
        name: "tick-test".to_owned(),
        view_distance: 10,
        spawn: Position::default(),
        game_mode: 0,
        connection: PlayerConnection {
            outbound,
            kick: Arc::new(KickFlag::default()),
            last_move: Arc::clone(&slot),
        },
    };
    region.insert(join);
    region
        .get_mut(3)
        .expect("the player is registered")
        .confirm_teleport();

    slot.store_input(protocol::PlayerInput::SNEAK | protocol::PlayerInput::SPRINT);
    slot.store_swing();
    let outcome = region.tick(0, std::time::Instant::now(), super::RegionGrid::new(2));
    assert!(outcome.moves.is_empty(), "no movement was sent");

    let player = region.get(3).expect("the player is registered");
    assert!(player.sneak());
    assert!(player.sprint());
    assert_eq!(player.swings(), 1);
}
