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
    // The punch of this tick was accepted: the swing is queued for the
    // visibility pass broadcast (M4.5 Шаг 11).
    assert_eq!(player.pending_swings(), 1);
    assert!(player.swinging());
}

/// Three punches in one tick all count as "just started": the rule accepts
/// every one of them (`swing_time < 0` after the first accept).
#[test]
fn three_punches_in_one_tick_produce_three_swings() {
    let slot = Arc::new(MoveSlot::default());
    let mut player = player(&slot);
    assert_eq!(player.apply_punches(3), 3);
    assert_eq!(player.pending_swings(), 3);
    assert!(player.swinging());
    assert_eq!(player.swing_time(), -1);
}

/// A punch during a young swing is dropped: the timer must reach
/// [`super::SWING_THRESHOLD_TICKS`] first.
#[test]
fn a_punch_during_a_young_swing_is_dropped() {
    let slot = Arc::new(MoveSlot::default());
    let mut player = player(&slot);
    assert_eq!(player.apply_punches(1), 1, "the first punch starts a swing");
    for expected in 0..super::SWING_THRESHOLD_TICKS {
        player.advance_swing();
        assert_eq!(player.swing_time(), expected);
        assert_eq!(
            player.apply_punches(1),
            0,
            "swing_time {expected} is below the threshold"
        );
    }
    // At the threshold the next punch is accepted again.
    player.advance_swing();
    assert_eq!(player.apply_punches(1), 1);
}

/// After the swing timer completes the swing ends and a punch is accepted
/// again without any threshold delay.
///
/// The timer starts at `-1` on the accept tick, so the reset at
/// [`super::SWING_DURATION_TICKS`] lands one advance later: the swing covers
/// the six ticks after the accept.
#[test]
fn a_punch_is_accepted_again_after_the_swing_ends() {
    let slot = Arc::new(MoveSlot::default());
    let mut player = player(&slot);
    assert_eq!(player.apply_punches(1), 1);
    for tick in 0..super::SWING_DURATION_TICKS {
        player.advance_swing();
        assert_eq!(
            player.swing_time(),
            tick,
            "the timer runs from 0 after the -1 start"
        );
    }
    // The next advance reaches the duration and resets the swing.
    player.advance_swing();
    assert!(!player.swinging(), "the swing ended when the timer reset");
    assert_eq!(player.swing_time(), 0, "the timer reset with the swing");
    assert_eq!(player.apply_punches(1), 1, "accepted immediately again");
}

/// The 22 `punch` moments of the vanilla S3 trace, run through the tick
/// model (phase 39.5 ms, delay 0, threshold 3), reproduce the observed
/// cluster structure `[1, 1, 1, 3, 3, 1, 1]` — 11 swings out of 22 punches.
///
/// The rule itself is a vanilla-memory hypothesis the trace confirms; the
/// threshold is **не определено трейсом** (3 and 4 both fit). Times are
/// relative to the first packet of A's connection, in ms; the model script
/// is `tools/trace-analyze/m45-step11-swing-model.py`.
// SOURCE: vanilla S3 trace (E:\\MineRust-data\\autotests\\2026-11-14_1644,
// mcproxy/proxy-conn-6.jsonl, 22 `punch` packets).
#[test]
fn the_s3_trace_punches_reproduce_the_cluster_structure() {
    const PUNCH_MS: [f64; 22] = [
        38579.1571, 39232.4664, 39880.8213, 40977.9975, 40979.1422, 41009.1126, 41084.9494,
        41085.2034, 41235.6302, 41236.3272, 41244.3545, 41278.7097, 41329.8760, 41383.9416,
        41430.5807, 41481.4416, 41527.0474, 41580.2678, 41624.9255, 41678.3446, 41732.5484,
        41781.4687,
    ];
    const PHASE_MS: f64 = 39.5;
    const TICK_MS: f64 = 50.0;

    // The punch moments mapped onto the tick grid, ascending.
    let first = ((PUNCH_MS[0] + PHASE_MS) / TICK_MS).floor() as i64;
    let last = ((PUNCH_MS[PUNCH_MS.len() - 1] + PHASE_MS) / TICK_MS).floor() as i64;
    let mut per_tick = vec![0u32; (last - first + 1) as usize];
    for punch in PUNCH_MS {
        let index = ((punch + PHASE_MS) / TICK_MS).floor() as i64 - first;
        per_tick[index as usize] += 1;
    }

    // The tick model: every tick runs, punch or not; the packets of the
    // tick are processed before the swing timer advances.
    let slot = Arc::new(MoveSlot::default());
    let mut player = player(&slot);
    let mut clusters: Vec<u32> = Vec::new();
    for count in per_tick {
        let accepted = player.apply_punches(count);
        if accepted > 0 {
            clusters.push(accepted);
        }
        player.advance_swing();
    }
    assert_eq!(clusters, vec![1, 1, 1, 3, 3, 1, 1], "accepted per tick");
    let total: u32 = clusters.iter().sum();
    assert_eq!(total, 11, "11 swings out of 22 punches");
}
