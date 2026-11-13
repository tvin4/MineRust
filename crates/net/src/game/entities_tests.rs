//! Tests of the M4.5 visibility pass and the entity-stream helpers.
//!
//! Socket-free: the pass is driven directly on hand-built regions, so a test
//! sees exactly what the tick would deliver.

use super::entities::{
    byte_angle, mutually_visible, pitch_byte, pose_differs, relative_delta, view_radius,
    visibility_pass, EntityMotion, EntityRemove, StreamEntities,
};
use super::test_support::join_at;
use super::{MoveUpdate, PlayerId, PlayerPos, Region, SINGLE_REGION};

/// A player pose with the given horizontal position and rotation.
fn pose(x: f64, z: f64, yaw: f32, pitch: f32) -> PlayerPos {
    PlayerPos {
        x,
        y: 64.0,
        z,
        yaw,
        pitch,
        flags: 0,
    }
}

/// One region with a confirmed player at each `(id, spawn)`.
fn region_with(players: &[(PlayerId, (i32, i32, i32))]) -> Region {
    let mut region = Region::new(SINGLE_REGION);
    for (id, spawn) in players {
        let (join, _slot, _receiver) = join_at(*id, *spawn);
        region.insert(join);
    }
    region
}

/// The single event of `viewer`, if any.
fn event_of(events: &[StreamEntities], viewer: PlayerId) -> Option<&StreamEntities> {
    events.iter().find(|event| event.player == viewer)
}

/// The ids of the spawned entities of one event.
fn spawned(events: &[StreamEntities], viewer: PlayerId) -> Vec<PlayerId> {
    event_of(events, viewer)
        .map(|event| event.spawn.iter().map(|s| s.id).collect())
        .unwrap_or_default()
}

fn removed(events: &[StreamEntities], viewer: PlayerId) -> Vec<EntityRemove> {
    event_of(events, viewer)
        .map(|event| event.remove.clone())
        .unwrap_or_default()
}

#[test]
fn the_radius_is_sixteen_blocks_per_chunk() {
    assert_eq!(view_radius(10), 160.0);
    assert_eq!(view_radius(0), 0.0);
    assert_eq!(view_radius(-3), 0.0, "a negative distance sees nothing");
}

#[test]
fn visibility_is_symmetric_and_uses_the_smaller_radius() {
    let a = pose(0.0, 0.0, 0.0, 0.0);
    let near = pose(50.0, 0.0, 0.0, 0.0);
    // One radius 160, one 64: the smaller one decides on both sides.
    assert!(mutually_visible(&a, 160.0, &near, 64.0));
    assert!(mutually_visible(&near, 64.0, &a, 160.0));
    assert_eq!(
        mutually_visible(&a, 160.0, &near, 64.0),
        mutually_visible(&near, 64.0, &a, 160.0),
        "the relation is symmetric"
    );
    // 100 blocks away is inside 160 but outside 64: the smaller radius wins.
    let far = pose(100.0, 0.0, 0.0, 0.0);
    assert!(!mutually_visible(&a, 160.0, &far, 64.0));
    assert!(!mutually_visible(&far, 64.0, &a, 160.0));
    // 50 blocks is outside a radius of 40 on both sides.
    assert!(!mutually_visible(&a, 40.0, &near, 40.0));
    // The circle is horizontal: only x and z matter.
    let above = PlayerPos { y: 999.0, ..near };
    assert!(mutually_visible(&a, 160.0, &above, 160.0));
}

#[test]
fn only_position_or_rotation_counts_as_a_change() {
    let base = pose(1.0, 2.0, 30.0, -10.0);
    assert!(!pose_differs(&base, &base));
    let flags_only = PlayerPos {
        flags: 0x03,
        ..base
    };
    assert!(
        !pose_differs(&base, &flags_only),
        "the flags byte is input state, not entity state"
    );
    assert!(pose_differs(&base, &pose(1.5, 2.0, 30.0, -10.0)));
    assert!(pose_differs(&base, &PlayerPos { yaw: 31.0, ..base }));
}

#[test]
fn byte_angles_match_the_trace() {
    // yaw 0xa0 is -135 degrees (round(-135 * 256 / 360) = -96 = 0xa0).
    assert_eq!(byte_angle(-135.0), 0xA0);
    assert_eq!(byte_angle(225.0), 0xA0, "wraps to the same angle");
    // pitch 0x03 is 4.2 degrees, 0xef is -17 (-23.9 degrees).
    assert_eq!(pitch_byte(4.2), 3);
    assert_eq!(pitch_byte(-23.9), -17);
}

#[test]
fn a_relative_delta_holds_up_to_eight_blocks() {
    assert_eq!(relative_delta(0.2), Some(819), "the traced 0.2-block step");
    assert_eq!(relative_delta(0.0), Some(0));
    assert!(relative_delta(8.0).is_none(), "8 blocks overflow an i16");
    assert_eq!(
        relative_delta(-8.0),
        Some(i16::MIN),
        "-8 blocks is exactly i16::MIN"
    );
    assert_eq!(relative_delta(7.9), Some((7.9_f64 * 4096.0).round() as i16));
}

#[test]
fn two_players_in_range_spawn_each_other() {
    let mut regions = vec![region_with(&[(1, (0, 64, 0)), (2, (50, 64, 0))])];
    let events = visibility_pass(&mut regions);

    assert_eq!(spawned(&events, 1), vec![2], "1 sees 2");
    assert_eq!(spawned(&events, 2), vec![1], "2 sees 1 (symmetry)");
    assert!(event_of(&events, 1).expect("event").moves.is_empty());
    assert_eq!(
        regions[0]
            .get(1)
            .expect("player")
            .visible()
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![2]
    );
    assert_eq!(
        regions[0]
            .get(2)
            .expect("player")
            .visible()
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        vec![1]
    );
}

#[test]
fn a_player_never_sees_himself() {
    let mut regions = vec![region_with(&[(1, (0, 64, 0))])];
    let events = visibility_pass(&mut regions);
    assert!(events.is_empty(), "a lone player has nothing to hear");
    assert!(regions[0].get(1).expect("player").visible().is_empty());
}

#[test]
fn players_out_of_range_do_not_see_each_other() {
    // Radius 160: 200 blocks apart is out of range.
    let mut regions = vec![region_with(&[(1, (0, 64, 0)), (2, (200, 64, 0))])];
    let events = visibility_pass(&mut regions);
    assert!(events.is_empty());
    assert!(regions[0].get(1).expect("player").visible().is_empty());
    assert!(regions[0].get(2).expect("player").visible().is_empty());
}

#[test]
fn leaving_the_radius_removes_the_entity_but_keeps_the_tab_entry() {
    let mut regions = vec![region_with(&[(1, (0, 64, 0)), (2, (50, 64, 0))])];
    visibility_pass(&mut regions);
    // Walk far away: no longer mutually visible.
    regions[0]
        .get_mut(2)
        .expect("player 2")
        .apply_move(&MoveUpdate {
            x: Some(300.0),
            y: None,
            z: None,
            yaw: None,
            pitch: None,
            flags: 0,
        });
    let events = visibility_pass(&mut regions);
    let removed_for_1 = removed(&events, 1);
    assert_eq!(removed_for_1.len(), 1);
    assert_eq!(removed_for_1[0].id, 2);
    assert!(
        !removed_for_1[0].left_world,
        "the player is still online, only out of range"
    );
    assert!(regions[0].get(1).expect("player").visible().is_empty());
    // The other side of the pair stops seeing 1 as well: symmetry.
    let removed_for_2 = removed(&events, 2);
    assert_eq!(removed_for_2.len(), 1);
    assert_eq!(removed_for_2[0].id, 1);
    assert!(
        !removed_for_2[0].left_world,
        "player 1 is still online, only out of range"
    );
}

#[test]
fn a_quit_removes_the_entity_and_the_tab_entry() {
    let mut regions = vec![region_with(&[(1, (0, 64, 0)), (2, (50, 64, 0))])];
    visibility_pass(&mut regions);
    // Player 2 leaves the world entirely.
    regions[0].remove(2).expect("player 2 was there");
    let events = visibility_pass(&mut regions);
    let removed = removed(&events, 1);
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].id, 2);
    assert!(removed[0].left_world, "the tab-list entry goes too");
    assert!(regions[0].get(1).expect("player").visible().is_empty());
}

#[test]
fn movement_goes_only_to_viewers_that_still_see_the_subject() {
    let mut regions = vec![region_with(&[(1, (0, 64, 0)), (2, (50, 64, 0))])];
    visibility_pass(&mut regions);
    regions[0]
        .get_mut(2)
        .expect("player 2")
        .apply_move(&MoveUpdate {
            x: Some(51.0),
            y: None,
            z: None,
            yaw: None,
            pitch: None,
            flags: 0,
        });
    let events = visibility_pass(&mut regions);
    let event = event_of(&events, 1).expect("viewer 1 hears about 2");
    assert_eq!(event.moves.len(), 1);
    assert_eq!(event.moves[0].id, 2);
    assert_eq!(
        event.moves[0].motion,
        EntityMotion::Position {
            dx: relative_delta(0.5).expect("0.5 blocks fit"),
            dy: 0,
            dz: 0,
            rotated: false,
        }
    );
    assert!(
        event_of(&events, 2).is_none(),
        "player 1 did not move, so 2 has nothing to hear"
    );
}

#[test]
fn a_rotation_only_change_is_reported_as_rotation() {
    let mut regions = vec![region_with(&[(1, (0, 64, 0)), (2, (50, 64, 0))])];
    visibility_pass(&mut regions);
    regions[0]
        .get_mut(2)
        .expect("player 2")
        .apply_move(&MoveUpdate {
            x: None,
            y: None,
            z: None,
            yaw: Some(90.0),
            pitch: Some(-45.0),
            flags: 0,
        });
    let events = visibility_pass(&mut regions);
    let event = event_of(&events, 1).expect("viewer 1");
    assert_eq!(event.moves.len(), 1);
    assert_eq!(event.moves[0].motion, EntityMotion::Rotation);
}

#[test]
fn a_large_jump_is_a_sync_not_a_relative_move() {
    let mut regions = vec![region_with(&[(1, (0, 64, 0)), (2, (50, 64, 0))])];
    visibility_pass(&mut regions);
    regions[0]
        .get_mut(2)
        .expect("player 2")
        .apply_move(&MoveUpdate {
            x: Some(90.0),
            y: None,
            z: None,
            yaw: None,
            pitch: None,
            flags: 0,
        });
    let events = visibility_pass(&mut regions);
    let event = event_of(&events, 1).expect("viewer 1");
    assert_eq!(
        event.moves[0].motion,
        EntityMotion::Sync,
        "40 blocks cannot be a relative move"
    );
}

/// A spawn keeps the shared baseline at the spawn position, so a later move of
/// a player that had no viewers is measured from the spawn, not from a stale
/// pose.
#[test]
fn a_spawn_resets_the_broadcast_baseline() {
    let mut regions = vec![region_with(&[(1, (0, 64, 0)), (2, (300, 64, 0))])];
    // No viewers yet: 2 is out of range.
    visibility_pass(&mut regions);
    // 2 moves while unseen.
    regions[0]
        .get_mut(2)
        .expect("player 2")
        .apply_move(&MoveUpdate {
            x: Some(20.0),
            y: None,
            z: None,
            yaw: None,
            pitch: None,
            flags: 0,
        });
    let events = visibility_pass(&mut regions);
    assert_eq!(
        spawned(&events, 1),
        vec![2],
        "2 is now in range and spawns at its current pose"
    );
    assert!(
        event_of(&events, 1).expect("event").moves.is_empty(),
        "the spawn carries the position, no separate move"
    );
    // The next move is measured from the spawn baseline, so it is small.
    regions[0]
        .get_mut(2)
        .expect("player 2")
        .apply_move(&MoveUpdate {
            x: Some(20.5),
            y: None,
            z: None,
            yaw: None,
            pitch: None,
            flags: 0,
        });
    let events = visibility_pass(&mut regions);
    let event = event_of(&events, 1).expect("viewer 1");
    assert_eq!(
        event.moves[0].motion,
        EntityMotion::Position {
            dx: relative_delta(0.5).expect("0.5 blocks fit"),
            dy: 0,
            dz: 0,
            rotated: false,
        }
    );
}

#[test]
fn the_radius_boundary_holds_on_several_radii_and_negative_coordinates() {
    let origin = pose(0.0, 0.0, 0.0, 0.0);
    for radius in [160.0, 64.0, 16.0, 8.0, 1.0] {
        let inside = pose(radius - 0.01, 0.0, 0.0, 0.0);
        let on = pose(radius, 0.0, 0.0, 0.0);
        let outside = pose(radius + 0.01, 0.0, 0.0, 0.0);
        assert!(
            mutually_visible(&origin, radius, &inside, radius),
            "inside r={radius}"
        );
        assert!(
            mutually_visible(&origin, radius, &on, radius),
            "exactly on the circle r={radius}"
        );
        assert!(
            !mutually_visible(&origin, radius, &outside, radius),
            "outside r={radius}"
        );
        // The same circle on the negative side of x.
        let neg_inside = pose(-(radius - 0.01), 0.0, 0.0, 0.0);
        let neg_outside = pose(-(radius + 0.01), 0.0, 0.0, 0.0);
        assert!(
            mutually_visible(&origin, radius, &neg_inside, radius),
            "negative inside r={radius}"
        );
        assert!(
            !mutually_visible(&origin, radius, &neg_outside, radius),
            "negative outside r={radius}"
        );
    }
    // Translation invariant: a negative-coordinate centre behaves the same.
    let centre = pose(-100.0, -100.0, 0.0, 0.0);
    assert!(mutually_visible(
        &centre,
        160.0,
        &pose(60.0, -100.0, 0.0, 0.0),
        160.0
    ));
    assert!(!mutually_visible(
        &centre,
        160.0,
        &pose(70.0, -100.0, 0.0, 0.0),
        160.0
    ));
    // A zero radius (a non-positive view distance) sees only its own point.
    assert!(mutually_visible(&origin, 0.0, &origin, 0.0));
    assert!(!mutually_visible(
        &origin,
        0.0,
        &pose(0.01, 0.0, 0.0, 0.0),
        0.0
    ));
}

#[test]
fn no_player_sees_itself_even_in_a_crowd() {
    let mut regions = vec![region_with(&[
        (1, (0, 64, 0)),
        (2, (5, 64, 0)),
        (3, (10, 64, 0)),
        (4, (15, 64, 0)),
    ])];
    let events = visibility_pass(&mut regions);
    assert_eq!(events.len(), 4, "every player hears about the others");
    for id in [1, 2, 3, 4] {
        let seen: Vec<PlayerId> = regions[0]
            .get(id)
            .expect("player")
            .visible()
            .keys()
            .copied()
            .collect();
        assert!(!seen.contains(&id), "player {id} must not see itself");
        assert_eq!(seen.len(), 3, "player {id} sees the other three");
    }
}
