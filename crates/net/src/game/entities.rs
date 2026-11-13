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

use std::collections::BTreeMap;

use protocol::{offline_uuid, Uuid};

use super::messages::PlayerId;
use super::region::{PlayerPos, Region, RegionId};

/// Blocks of horizontal view radius per chunk of view distance.
pub const BLOCKS_PER_CHUNK: f64 = 16.0;

/// Server ticks between two pose broadcasts of one subject (M4.5 Шаг 6.2).
///
/// Vanilla updates a subject once per two ticks: 92/103 gaps between
/// consecutive updates are 2 B-ticks in С2 and 47/55 in С4.
// SOURCE: two-bot vanilla trace 26.3 (С2/С4), per-tick tables in §6.
pub const UPDATE_INTERVAL: u32 = 2;

/// Ticks without a pose change after which a zero-delta `move_entity_pos` is
/// broadcast (M4.5 Шаг 6.2).
///
/// С1 sends it every 2.98/3.00 s (about 60 ticks) and С2 also 2996 ms; С4 is
/// irregular, so the exact periodicity is «не установлено» and 60 is used.
// SOURCE: two-bot vanilla trace 26.3 (С1/С2), §6.
pub const BROADCAST_PERIOD_TICKS: u32 = 60;

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

/// A yaw in degrees as the protocol's one-byte angle
/// (`floor(deg · 256 / 360)`).
///
/// Vanilla floors the angle in f64 (С2: floor 0/68 vs round 33/68; С4: 0/51 vs
/// 42/51); rounding was our earlier defect.
// SOURCE: two-bot vanilla trace 26.3 (С2/С4), §6.
pub fn byte_angle(degrees: f32) -> u8 {
    (f64::from(degrees) * (256.0 / 360.0)).floor() as i32 as u8
}

/// A pitch in degrees as the protocol's one signed-byte angle
/// (`floor(deg · 256 / 360)`, two's complement).
// SOURCE: two-bot vanilla trace 26.3 (С2/С4), §6.
pub fn pitch_byte(degrees: f32) -> i8 {
    (f64::from(degrees) * (256.0 / 360.0)).floor() as i32 as i8
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
    /// Game mode of the spawned player (M4.5 Шаг 7.3): the tab-list entry's
    /// game mode and the mode-dependent metadata bytes come from it.
    pub game_mode: i32,
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
    /// No pose change for a whole broadcast period: a zero-delta
    /// `move_entity_pos`, sent so the client's position stays fresh (M4.5 Шаг
    /// 6.2).
    ZeroDelta,
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
            Self::ZeroDelta => "zero_delta",
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

/// The spawn record of a newly visible player: its name and offline UUID are
/// read from the player state only when a player actually becomes visible,
/// never per tick for everyone.
fn spawn_of(region: &Region, id: PlayerId, position: PlayerPos) -> EntitySpawn {
    // The id is in this tick's snapshot, so the player exists; the fallback is
    // defensive only and is never hit.
    let (name, uuid, game_mode) = match region.get(id) {
        Some(player) => (
            player.name().to_owned(),
            offline_uuid(player.name()),
            player.game_mode(),
        ),
        None => (String::new(), Uuid::default(), 0),
    };
    EntitySpawn {
        id,
        name,
        uuid,
        position,
        game_mode,
    }
}

/// Computes this tick's visibility difference for every region and updates the
/// players' remembered visible sets and broadcast baselines.
///
/// Runs after the transfers, so `regions` is the final owner of every player.
/// The result is one event per viewer that has something to hear, ordered by
/// region then viewer id; empty events are dropped.
///
/// Cost is O(P²) pairwise comparisons plus O(P + visible pairs) work for the
/// events and the write-back: the positions, radii and baselines are snapshotted
/// into `Vec`s once, the pairwise loop and the per-viewer set operations touch
/// only those `Vec`s (no `BTreeMap` inside a hot loop), and every player's
/// visible set is updated in place rather than rebuilt. Names and UUIDs are
/// read only for the players that actually become visible.
pub(crate) fn visibility_pass(regions: &mut [Region]) -> Vec<StreamEntities> {
    // Snapshot everything once, in (region, player id) order — the order the
    // events are emitted in. `index` maps an id to its dense snapshot index;
    // `sorted` lists the indices in ascending `PlayerId`, so the pairwise loop
    // and the merged set operations stay in ascending-id order.
    let mut ids: Vec<PlayerId> = Vec::new();
    let mut position: Vec<PlayerPos> = Vec::new();
    let mut radius: Vec<f64> = Vec::new();
    let mut owner: Vec<RegionId> = Vec::new();
    let mut owner_region: Vec<usize> = Vec::new();
    let mut baseline: Vec<Option<PlayerPos>> = Vec::new();
    let mut ticks: Vec<u32> = Vec::new();
    let mut before: Vec<Vec<PlayerId>> = Vec::new();
    let mut index: BTreeMap<PlayerId, usize> = BTreeMap::new();
    for (region_index, region) in regions.iter().enumerate() {
        for id in region.player_ids() {
            let Some(player) = region.get(id) else {
                continue;
            };
            index.insert(id, ids.len());
            ids.push(id);
            position.push(player.position());
            radius.push(view_radius(player.view_distance()));
            owner.push(region.id());
            owner_region.push(region_index);
            baseline.push(player.last_broadcast());
            ticks.push(player.ticks_since_broadcast());
            before.push(player.visible().keys().copied().collect());
        }
    }
    if ids.is_empty() {
        return Vec::new();
    }
    let sorted: Vec<usize> = index.values().copied().collect();

    // The symmetric target: `a` sees `b` iff the distance is within the
    // smaller radius. Self is never included. `target[a]` is ascending by id.
    let mut target: Vec<Vec<usize>> = vec![Vec::new(); ids.len()];
    for &a in &sorted {
        let pose_a = position[a];
        let radius_a = radius[a];
        let seen = &mut target[a];
        for &b in &sorted {
            if a != b && mutually_visible(&pose_a, radius_a, &position[b], radius[b]) {
                seen.push(b);
            }
        }
    }

    // Movement of every subject, shared by all its viewers. Vanilla broadcasts
    // a subject once per `UPDATE_INTERVAL` ticks and a zero-delta position
    // packet after `BROADCAST_PERIOD_TICKS` without a change (Шаг 6.2). A moved
    // subject resets the baseline every viewer measures the next move from, and
    // so does a subject that just became visible (its spawn carries the
    // position).
    let mut motion: Vec<Option<EntityMotion>> = vec![None; ids.len()];
    let mut next_baseline = baseline.clone();
    let mut next_ticks: Vec<u32> = ticks.clone();
    for &subject in &sorted {
        let counter = ticks[subject].saturating_add(1);
        next_ticks[subject] = counter;
        let Some(previous) = baseline[subject] else {
            continue;
        };
        let now = position[subject];
        if pose_differs(&previous, &now) {
            if counter >= UPDATE_INTERVAL {
                motion[subject] = Some(motion_from(&previous, &now));
                next_baseline[subject] = Some(now);
                next_ticks[subject] = 0;
            }
        } else if counter >= BROADCAST_PERIOD_TICKS {
            motion[subject] = Some(EntityMotion::ZeroDelta);
            next_ticks[subject] = 0;
        }
    }

    // One event per viewer, in snapshot order. `target[i]` (new) and
    // `before[i]` (old) are both ascending by id, so a two-pointer merge gives
    // spawn, remove and moves without any set or map lookup. The spawned
    // entries are also kept for the in-place write-back below.
    let mut events = Vec::new();
    let mut spawn_delta: Vec<Vec<(PlayerId, Uuid)>> = vec![Vec::new(); ids.len()];
    let mut remove_delta: Vec<Vec<PlayerId>> = vec![Vec::new(); ids.len()];
    for i in 0..ids.len() {
        let now = &target[i];
        let old = &before[i];
        let mut spawn = Vec::new();
        let mut remove = Vec::new();
        let mut moves = Vec::new();
        let (mut n, mut o) = (0usize, 0usize);
        while n < now.len() || o < old.len() {
            let now_id = now.get(n).map(|&j| ids[j]);
            let old_id = old.get(o).copied();
            match (now_id, old_id) {
                (Some(left), Some(right)) if left == right => {
                    // Still visible: a move only if the subject moved.
                    if let Some(motion) = motion[now[n]] {
                        moves.push(EntityMove {
                            id: left,
                            position: position[now[n]],
                            motion,
                        });
                    }
                    n += 1;
                    o += 1;
                }
                (Some(left), Some(right)) if left < right => {
                    let j = now[n];
                    let record = spawn_of(&regions[owner_region[j]], ids[j], position[j]);
                    spawn_delta[i].push((left, record.uuid));
                    spawn.push(record);
                    next_baseline[j] = Some(position[j]);
                    next_ticks[j] = 0;
                    n += 1;
                }
                (Some(_), Some(_)) => {
                    let gone = old[o];
                    let uuid = regions[owner_region[i]]
                        .get(ids[i])
                        .and_then(|player| player.visible().get(&gone).copied())
                        .unwrap_or_default();
                    remove.push(EntityRemove {
                        id: gone,
                        uuid,
                        left_world: !index.contains_key(&gone),
                    });
                    remove_delta[i].push(gone);
                    o += 1;
                }
                (Some(_), None) => {
                    let j = now[n];
                    let record = spawn_of(&regions[owner_region[j]], ids[j], position[j]);
                    spawn_delta[i].push((ids[j], record.uuid));
                    spawn.push(record);
                    next_baseline[j] = Some(position[j]);
                    next_ticks[j] = 0;
                    n += 1;
                }
                (None, Some(_)) => {
                    let gone = old[o];
                    let uuid = regions[owner_region[i]]
                        .get(ids[i])
                        .and_then(|player| player.visible().get(&gone).copied())
                        .unwrap_or_default();
                    remove.push(EntityRemove {
                        id: gone,
                        uuid,
                        left_world: !index.contains_key(&gone),
                    });
                    remove_delta[i].push(gone);
                    o += 1;
                }
                (None, None) => break,
            }
        }
        if spawn.is_empty() && remove.is_empty() && moves.is_empty() {
            continue;
        }
        events.push(StreamEntities {
            region: owner[i],
            player: ids[i],
            // The entity pass runs after the tick's own events; `u32::MAX`
            // sorts it after them inside the region (see `TickOutcome`).
            seq: u32::MAX,
            spawn,
            remove,
            moves,
        });
    }

    // Write the new visible sets back in place: drop the gone entries, add the
    // newly visible ones with the UUIDs read at spawn time. Stable entries are
    // left untouched, so no map is rebuilt per tick.
    for region in regions.iter_mut() {
        let region_ids: Vec<PlayerId> = region.player_ids().collect();
        for id in region_ids {
            let Some(&i) = index.get(&id) else {
                continue;
            };
            let Some(player) = region.get_mut(id) else {
                continue;
            };
            let visible = player.visible_mut();
            for gone in &remove_delta[i] {
                visible.remove(gone);
            }
            for (new_id, uuid) in &spawn_delta[i] {
                visible.insert(*new_id, *uuid);
            }
            player.set_last_broadcast(next_baseline[i]);
            player.set_ticks_since_broadcast(next_ticks[i]);
        }
    }
    events
}
