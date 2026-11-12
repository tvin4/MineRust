//! Player visibility and the M4.5 entity stream (M4.5 Шаг 3).
//!
//! Visibility is a **horizontal circle** of radius `view_distance × 16` blocks
//! around each player, with **no hysteresis**, and it is **symmetric**: the
//! same decision is made for both members of a pair, using the smaller of the
//! two radii (`min(r_a, r_b)`). A player never sees itself.
//!
//! [`visibility_pass`] runs once per tick, after the scheduler applied this
//! tick's transfers, so it sees the final ownership and positions. It turns
//! the difference between last tick's visibility and this tick's into one
//! [`StreamEntities`] per viewer that has something to hear: entities to spawn,
//! entities to remove and movement updates. Which packets and in which order
//! is the connection's business; this module only says *what changed*.
//!
//! The spawn shape and the packet choices follow the two-bot vanilla trace
//! (С1–С4, `docs/ARCH-M4.md` §6). Where the trace does not establish a value
//! it is carried explicitly and marked «не установлено» — nothing is guessed.
//!
//! SOURCE: two-bot vanilla trace of 26.3 (byte layout observed), not wiki.

use std::collections::{BTreeMap, BTreeSet};

use protocol::{offline_uuid, Uuid};

use super::messages::PlayerId;
use super::region::{PlayerPos, Region, RegionId};

/// Blocks of horizontal view radius per chunk of view distance.
pub const BLOCKS_PER_CHUNK: f64 = 16.0;

/// Entity type id of a player in `add_entity`.
// SOURCE: two-bot vanilla trace 26.3 (C1–C4): type 159 for the traced player.
pub const PLAYER_ENTITY_TYPE: i32 = 159;

/// The horizontal radius a client with this view distance sees, in blocks.
pub fn view_radius(view_distance: i32) -> f64 {
    f64::from(view_distance.max(0)) * BLOCKS_PER_CHUNK
}

/// Whether two players are mutually visible: the horizontal distance between
/// them is at most the **smaller** of the two view radii.
///
/// Using the smaller radius on both sides makes the relation symmetric by
/// construction, so `a` sees `b` exactly when `b` sees `a`.
pub fn mutually_visible(a: &PlayerPos, radius_a: f64, b: &PlayerPos, radius_b: f64) -> bool {
    let dx = a.x - b.x;
    let dz = a.z - b.z;
    let radius = radius_a.min(radius_b);
    dx * dx + dz * dz <= radius * radius
}

/// Whether two poses differ in position or rotation.
///
/// The flags byte is input state (on-ground), not entity state, so it is
/// ignored: a change of flags alone must not produce an entity packet.
pub fn pose_differs(a: &PlayerPos, b: &PlayerPos) -> bool {
    a.x != b.x || a.y != b.y || a.z != b.z || a.yaw != b.yaw || a.pitch != b.pitch
}

/// A yaw in degrees as the protocol's one-byte angle (`round(deg · 256 / 360)`).
pub fn byte_angle(degrees: f32) -> u8 {
    (degrees * (256.0 / 360.0)).round() as i32 as u8
}

/// A pitch in degrees as the protocol's one signed-byte angle.
pub fn pitch_byte(degrees: f32) -> i8 {
    (degrees * (256.0 / 360.0)).round() as i32 as i8
}

/// A relative delta in 1/4096 blocks, or `None` when it does not fit an `i16`
/// (about ±8 blocks): such a jump needs [`EntityMotion::Sync`] instead.
pub fn relative_delta(delta: f64) -> Option<i16> {
    let scaled = (delta * 4096.0).round();
    if scaled >= f64::from(i16::MIN) && scaled <= f64::from(i16::MAX) {
        Some(scaled as i16)
    } else {
        None
    }
}

/// An entity a viewer starts seeing this tick.
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySpawn {
    /// Entity (player) id.
    pub id: PlayerId,
    /// Name shown in the tab list.
    pub name: String,
    /// Offline UUID of the player.
    pub uuid: Uuid,
    /// Position at the moment it became visible.
    pub position: PlayerPos,
}

/// An entity a viewer stops seeing this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityRemove {
    /// Entity (player) id.
    pub id: PlayerId,
    /// Offline UUID, kept so the connection can drop the tab-list entry even
    /// after the player left the world.
    pub uuid: Uuid,
    /// Whether the player left the world entirely (then the tab list entry is
    /// removed too) rather than only leaving the view radius.
    pub left_world: bool,
}

/// How an entity's pose changed since the last broadcast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityMotion {
    /// A relative position move, in 1/4096 blocks; `rotated` also carries
    /// yaw and pitch.
    Position {
        /// X delta.
        dx: i16,
        /// Y delta.
        dy: i16,
        /// Z delta.
        dz: i16,
        /// Whether the body rotation changed as well.
        rotated: bool,
    },
    /// A rotation-only change.
    Rotation,
    /// A jump too large for the relative packets; the absolute position is
    /// sent instead.
    Sync,
}

impl EntityMotion {
    /// Stable name for the transcript and logs.
    pub fn name(self) -> &'static str {
        match self {
            Self::Position { .. } => "position",
            Self::Rotation => "rotation",
            Self::Sync => "sync",
        }
    }

    /// Whether this motion also carries the body rotation.
    pub fn rotated(self) -> bool {
        matches!(self, Self::Position { rotated: true, .. })
    }
}

/// A movement update for one entity, as one viewer must receive it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntityMove {
    /// Entity (player) id.
    pub id: PlayerId,
    /// The absolute position after the move.
    pub position: PlayerPos,
    /// What changed.
    pub motion: EntityMotion,
}

/// Everything one viewer must be told about other players this tick.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamEntities {
    /// Region that owns the viewer after this tick's transfers.
    pub region: RegionId,
    /// The viewer the packets go to.
    pub player: PlayerId,
    /// Order of the event inside the region; the transcript uses
    /// [`u32::MAX`] so entity events sort after the tick's own results.
    pub seq: u32,
    /// Entities to spawn, in ascending id.
    pub spawn: Vec<EntitySpawn>,
    /// Entities to remove, in ascending id.
    pub remove: Vec<EntityRemove>,
    /// Movement updates, in ascending id.
    pub moves: Vec<EntityMove>,
}

impl StreamEntities {
    /// Whether this event carries nothing and can be skipped.
    pub fn is_empty(&self) -> bool {
        self.spawn.is_empty() && self.remove.is_empty() && self.moves.is_empty()
    }
}

/// The entity motion between two poses; only called when they differ.
fn motion_from(previous: &PlayerPos, now: &PlayerPos) -> EntityMotion {
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

/// Computes this tick's visibility difference for every region and updates the
/// players' remembered visible sets and broadcast baselines.
///
/// Runs after the transfers, so `regions` is the final owner of every player.
/// The result is one event per viewer that has something to hear, ordered by
/// region then viewer id; empty events are dropped.
pub(crate) fn visibility_pass(regions: &mut [Region]) -> Vec<StreamEntities> {
    // Snapshot everything once, so the write-back below is the only mutation.
    let mut ids: Vec<PlayerId> = Vec::new();
    let mut position: BTreeMap<PlayerId, PlayerPos> = BTreeMap::new();
    let mut radius: BTreeMap<PlayerId, f64> = BTreeMap::new();
    let mut owner: BTreeMap<PlayerId, RegionId> = BTreeMap::new();
    let mut name: BTreeMap<PlayerId, String> = BTreeMap::new();
    let mut uuid: BTreeMap<PlayerId, Uuid> = BTreeMap::new();
    let mut visible_before: BTreeMap<PlayerId, BTreeMap<PlayerId, Uuid>> = BTreeMap::new();
    let mut baseline: BTreeMap<PlayerId, Option<PlayerPos>> = BTreeMap::new();
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
            visible_before.insert(id, player.visible().clone());
            baseline.insert(id, player.last_broadcast());
        }
    }
    if ids.is_empty() {
        return Vec::new();
    }

    // The symmetric target: `a` sees `b` iff the distance is within the
    // smaller radius. Self is never included.
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

    // Movement goes to viewers that saw the subject last tick and still do:
    // a newly seeing viewer gets the position from the spawn instead, and a
    // leaving viewer gets a removal.
    let mut moves: BTreeMap<PlayerId, Vec<EntityMove>> = BTreeMap::new();
    let mut next_baseline = baseline.clone();
    for &subject in &ids {
        let Some(previous) = baseline[&subject] else {
            continue;
        };
        let now = position[&subject];
        if !pose_differs(&previous, &now) {
            continue;
        }
        let motion = motion_from(&previous, &now);
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
        next_baseline.insert(subject, Some(now));
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
                });
                // The new viewer starts at the current position, so the shared
                // baseline must move to it as well.
                next_baseline.insert(id, Some(position[&id]));
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

        // The viewer keeps its own record of who it sees, with each UUID, so a
        // later removal works even after the other player left the world.
        let mut record = BTreeMap::new();
        for &id in now {
            record.insert(id, uuid[&id]);
        }
        visible_after.insert(viewer, record);

        let event = StreamEntities {
            region: owner[&viewer],
            player: viewer,
            // The entity pass runs after the tick's own events; `u32::MAX`
            // sorts it after them inside the region (see `TickOutcome`).
            seq: u32::MAX,
            spawn,
            remove,
            moves: viewer_moves,
        };
        if !event.is_empty() {
            events.push(event);
        }
    }

    // Write the new state back: the visible set and the broadcast baseline.
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
        }
    }
    events
}
