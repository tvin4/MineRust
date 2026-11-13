//! The previous, naive visibility pass, kept **verbatim** as the reference of
//! the M4.5 Шаг 3.5 equivalence test, plus the Шаг 6.2 broadcast cadence.
//!
//! The optimized [`super::entities::visibility_pass`] must produce exactly the
//! same visible sets as this one for the same input; the random-layout test in
//! `entities_equivalence_tests.rs` checks that. This module is test-only and
//! must not be used by production code.
//!
//! SOURCE: the M4.5 Шаг 3 implementation (`6c1a850`), with the Шаг 6.2
//! per-subject counter (`UPDATE_INTERVAL`, `BROADCAST_PERIOD_TICKS`).

use std::collections::{BTreeMap, BTreeSet};

use protocol::{offline_uuid, Uuid};

use super::entities::{
    mutually_visible, pose_differs, relative_delta, view_radius, EntityMotion, EntityMove,
    EntityRemove, EntitySpawn, StreamEntities, BROADCAST_PERIOD_TICKS, UPDATE_INTERVAL,
};
use super::messages::PlayerId;
use super::region::{PlayerPos, Region, RegionId};

/// The entity motion between two poses; only called when they differ.
fn reference_motion_from(previous: &PlayerPos, now: &PlayerPos) -> EntityMotion {
    let dx = now.x - previous.x;
    let dy = now.y - previous.y;
    let dz = now.z - previous.z;
    if dx == 0.0 && dy == 0.0 && dz == 0.0 {
        return EntityMotion::Rotation;
    }
    let rotated = now.yaw != previous.yaw || now.pitch != previous.pitch;
    match (relative_delta(dx), relative_delta(dy), relative_delta(dz)) {
        (Some(dx), Some(dy), Some(dz)) => EntityMotion::Position {
            dx,
            dy,
            dz,
            rotated,
        },
        _ => EntityMotion::Sync,
    }
}

/// The naive O(P² log P) pass of M4.5 Шаг 3: every comparison goes through
/// `BTreeMap`s, every visible set is rebuilt each tick.
pub(crate) fn naive_visibility_pass(regions: &mut [Region]) -> Vec<StreamEntities> {
    let mut ids: Vec<PlayerId> = Vec::new();
    let mut position: BTreeMap<PlayerId, PlayerPos> = BTreeMap::new();
    let mut radius: BTreeMap<PlayerId, f64> = BTreeMap::new();
    let mut owner: BTreeMap<PlayerId, RegionId> = BTreeMap::new();
    let mut name: BTreeMap<PlayerId, String> = BTreeMap::new();
    let mut uuid: BTreeMap<PlayerId, Uuid> = BTreeMap::new();
    let mut game_mode: BTreeMap<PlayerId, i32> = BTreeMap::new();
    let mut visible_before: BTreeMap<PlayerId, BTreeMap<PlayerId, Uuid>> = BTreeMap::new();
    let mut baseline: BTreeMap<PlayerId, Option<PlayerPos>> = BTreeMap::new();
    let mut ticks: BTreeMap<PlayerId, u32> = BTreeMap::new();
    for region in regions.iter() {
        for id in region.player_ids() {
            let Some(player) = region.get(id) else {
                continue;
            };
            ids.push(id);
            position.insert(id, player.position());
            radius.insert(id, view_radius(player.view_distance()));
            owner.insert(id, region.id());
            name.insert(id, player.name().to_owned());
            uuid.insert(id, offline_uuid(player.name()));
            game_mode.insert(id, player.game_mode());
            visible_before.insert(id, player.visible().clone());
            baseline.insert(id, player.last_broadcast());
            ticks.insert(id, player.ticks_since_broadcast());
        }
    }
    if ids.is_empty() {
        return Vec::new();
    }

    let mut target: BTreeMap<PlayerId, BTreeSet<PlayerId>> = BTreeMap::new();
    for &a in &ids {
        let mut seen = BTreeSet::new();
        for &b in &ids {
            if a != b && mutually_visible(&position[&a], radius[&a], &position[&b], radius[&b]) {
                seen.insert(b);
            }
        }
        target.insert(a, seen);
    }

    let mut moves: BTreeMap<PlayerId, Vec<EntityMove>> = BTreeMap::new();
    let mut next_baseline = baseline.clone();
    let mut next_ticks = ticks.clone();
    for &subject in &ids {
        let counter = ticks[&subject].saturating_add(1);
        next_ticks.insert(subject, counter);
        let Some(previous) = baseline[&subject] else {
            continue;
        };
        let now = position[&subject];
        let changed = pose_differs(&previous, &now);
        let motion = if changed && counter >= UPDATE_INTERVAL {
            next_baseline.insert(subject, Some(now));
            reference_motion_from(&previous, &now)
        } else if !changed && counter >= BROADCAST_PERIOD_TICKS {
            EntityMotion::ZeroDelta
        } else {
            continue;
        };
        next_ticks.insert(subject, 0);
        for &viewer in &ids {
            if viewer != subject
                && visible_before[&viewer].contains_key(&subject)
                && target[&viewer].contains(&subject)
            {
                moves.entry(viewer).or_default().push(EntityMove {
                    id: subject,
                    position: now,
                    motion,
                });
            }
        }
    }

    let mut events = Vec::new();
    let mut visible_after: BTreeMap<PlayerId, BTreeMap<PlayerId, Uuid>> = BTreeMap::new();
    for &viewer in &ids {
        let before = &visible_before[&viewer];
        let now = &target[&viewer];
        let mut spawn = Vec::new();
        for &id in now {
            if !before.contains_key(&id) {
                spawn.push(EntitySpawn {
                    id,
                    name: name[&id].clone(),
                    uuid: uuid[&id],
                    position: position[&id],
                    game_mode: game_mode[&id],
                });
                next_baseline.insert(id, Some(position[&id]));
                next_ticks.insert(id, 0);
            }
        }
        let mut remove = Vec::new();
        for (&id, &entity_uuid) in before {
            if !now.contains(&id) {
                remove.push(EntityRemove {
                    id,
                    uuid: entity_uuid,
                    left_world: !position.contains_key(&id),
                });
            }
        }
        let mut viewer_moves = moves.remove(&viewer).unwrap_or_default();
        viewer_moves.sort_by_key(|entity| entity.id);

        let mut record = BTreeMap::new();
        for &id in now {
            record.insert(id, uuid[&id]);
        }
        visible_after.insert(viewer, record);

        let event = StreamEntities {
            region: owner[&viewer],
            player: viewer,
            seq: u32::MAX,
            spawn,
            remove,
            moves: viewer_moves,
        };
        if !event.is_empty() {
            events.push(event);
        }
    }

    for region in regions.iter_mut() {
        let region_ids: Vec<PlayerId> = region.player_ids().collect();
        for id in region_ids {
            let Some(player) = region.get_mut(id) else {
                continue;
            };
            if let Some(visible) = visible_after.remove(&id) {
                player.set_visible(visible);
            }
            if let Some(broadcast) = next_baseline.get(&id) {
                player.set_last_broadcast(*broadcast);
            }
            if let Some(ticks) = next_ticks.get(&id) {
                player.set_ticks_since_broadcast(*ticks);
            }
        }
    }
    events
}
