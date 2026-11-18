//! Block placement from `use_item_on` and the creative pick (Шаг 24b/24a).
//!
//! The rules come from the S17 and S24b vanilla traces
//! (`docs/BLOCK-PLACING.md` §3–§5):
//!
//! - the target cell is the clicked cell plus the clicked face;
//! - the reach is `5.0` in creative and `4.5` otherwise, measured from the
//!   eyes to the hit point the packet carries (the S24b sweep hit the face at
//!   `4.5` and missed at `5.0`, so the client's own boundary is exclusive; the
//!   server's check accepts the hit point at exactly `5.0`);
//! - a block that would intersect a player's box is refused: the actor gets
//!   `block_update` of the clicked block, `block_update` of the untouched
//!   target and the ack, observers get nothing (the S24b own-cell case);
//! - air, water, short grass and snow are replaced; anything else is not
//!   (S17 `replace-*` cases);
//! - a log takes its `axis` from the clicked face (S17 `log-face`: face north
//!   leaves `axis=z`); a slab takes `type` from the cursor height; stairs take
//!   `facing` from the player's yaw and `half` from the cursor height, with
//!   the neighbour rule of §24.0d (only `outer_left` is measured); a
//!   waterloggable block placed into water gets `waterlogged=true` (S24b
//!   waterlogged stair);
//! - the actor sees `block_update` of the clicked block, then the target's
//!   new state **twice**, then the ack; every other player sees the target's
//!   new state once (S24b per-connection streams).
//!
//! Not modelled (debt, `docs/BLOCK-PLACING.md` §6): light and heightmap
//! updates (the S24b trace shows no `light_update` for a placed block in its
//! own chunk), placement sounds and particles (none in the filtered trace),
//! and a replacement rule for flowers/tall grass, which the trace does not
//! carry. A block whose item identifier differs from the block identifier
//! (`redstone` → `redstone_wire`, torches on a wall, …) is picked only when
//! the two names match; the exceptions are debt.

use std::sync::Arc;

use data::{BlockStates, Properties, RegistryIds};
use protocol::{ItemStack, Position};
use tracing::{debug, warn};

use crate::store::ChunkStore;

use super::block_action::deliver;
use super::inventory::ITEM_REGISTRY;
use super::messages::{BlockPlacement, PlayerId, ServerMessage};
use super::replay::Replay;
use super::scheduler::Scheduler;

/// Game mode id of creative (the mode with the `4.5 + 0.5` block reach).
pub const CREATIVE: i32 = 1;
/// Block interaction range of creative, from the eyes.
pub const CREATIVE_REACH: f64 = 5.0;
/// Block interaction range of every other mode, from the eyes.
pub const SURVIVAL_REACH: f64 = 4.5;
/// Height of the vanilla player's eyes above the feet.
pub const EYE_HEIGHT: f64 = 1.62;
/// Half width of the player's bounding box.
const PLAYER_HALF_WIDTH: f64 = 0.3;
/// Height of the player's bounding box.
const PLAYER_HEIGHT: f64 = 1.8;

/// The block names a placement may replace.
///
/// SOURCE: the S17 trace's `replace-short-grass`, `replace-snow` and
/// `replace-water` cases (the placed stone won the cell), plus the three air
/// blocks the report and the break code already use. Flowers and tall grass
/// are **not** in the trace; they stay debt instead of being guessed at.
const REPLACEABLE: [&str; 3] = ["minecraft:water", "minecraft:short_grass", "minecraft:snow"];

/// Applies one creative pick (Шаг 24a): the selected slot and, when the item
/// was not in the hotbar, the new slot content.
///
/// The S17 answer order is `set_held_slot` first, then `container_set_slot`.
/// The item of a block is the item with the same identifier; a block without
/// such an item sends nothing (debt note in the module docs).
pub(super) fn pick<W: std::io::Write>(
    scheduler: &mut Scheduler,
    replay: &mut Replay<W>,
    tick: u64,
    id: PlayerId,
    position: Position,
    store: Option<&Arc<ChunkStore>>,
    items: Option<&RegistryIds>,
) {
    replay.record(
        tick,
        scheduler.region_of(id),
        id,
        "pick_item",
        serde_json::json!({ "x": position.x, "y": position.y, "z": position.z }),
    );
    let (Some(store), Some(items)) = (store, items) else {
        debug!("pick ignored: no world or item registry");
        return;
    };
    let blocks = store.blocks();
    let Some(state) = store.block(position.x, position.y, position.z) else {
        debug!(y = position.y, "pick ignored: the chunk is not loaded");
        return;
    };
    let Some(name) = blocks.block_name(state) else {
        debug!(
            state,
            "pick ignored: the block report does not know the state"
        );
        return;
    };
    let Some(item) = items.entry_id(ITEM_REGISTRY, name) else {
        debug!(block = name, "pick ignored: no item with the block's name");
        return;
    };
    let stack = ItemStack::new(item, 1);
    let Some(player) = scheduler.player_mut(id) else {
        return;
    };
    let picked = player.inventory_mut().pick(stack.clone());
    // The borrow ends here: the answer is sent after the model changed.
    deliver(
        scheduler,
        id,
        ServerMessage::SetHeldSlot { slot: picked.held },
    );
    if let Some((slot, state_id)) = picked.added {
        deliver(
            scheduler,
            id,
            ServerMessage::ContainerSetSlot {
                state_id,
                slot,
                item: stack,
            },
        );
    }
}

/// Applies one `use_item_on` placement request (Шаг 24b).
pub(super) fn place<W: std::io::Write>(
    scheduler: &mut Scheduler,
    replay: &mut Replay<W>,
    tick: u64,
    action: BlockPlacement,
    store: Option<&Arc<ChunkStore>>,
    blocks: Option<&BlockStates>,
    items: Option<&RegistryIds>,
) {
    let BlockPlacement {
        id,
        hand,
        position,
        face,
        cursor,
        inside_block,
        world_border,
        sequence,
    } = action;
    replay.record(
        tick,
        scheduler.region_of(id),
        id,
        "use_item_on",
        serde_json::json!({
            "hand": hand,
            "x": position.x,
            "y": position.y,
            "z": position.z,
            "face": face,
            "cursor": [cursor.0, cursor.1, cursor.2],
            "inside_block": inside_block,
            "world_border": world_border,
            "sequence": sequence,
        }),
    );
    let (Some(store), Some(blocks), Some(items)) = (store, blocks, items) else {
        ack(scheduler, id, sequence);
        return;
    };
    let Some(hit_state) = store.block(position.x, position.y, position.z) else {
        ack(scheduler, id, sequence);
        return;
    };
    // The cell behind the clicked face is what the client's prediction marks;
    // the actual destination may be the clicked cell itself.
    let Some(predicted) = neighbour(position, face) else {
        ack(scheduler, id, sequence);
        return;
    };
    // A replaceable clicked cell is replaced in place: the S17
    // `replace-short-grass` click names the grass cell (its outline stops the
    // crosshair before the support), and vanilla's stone lands in that cell.
    // SOURCE: 2026-11-18_s17-vanilla-step23-r2: `use_item_on` seq 80534
    // pos=(104,202,-21) face=2, then the actor's block_update
    // (104,202,-21)=stone, (104,202,-22)=air, (104,202,-21)=stone and ack;
    // the observer sees (104,202,-21)=stone once.
    let clicked_replaceable = is_replaceable(blocks, hit_state);
    let (target, target_state) = if clicked_replaceable {
        (position, hit_state)
    } else {
        let Some(state) = store.block(predicted.x, predicted.y, predicted.z) else {
            ack(scheduler, id, sequence);
            return;
        };
        (predicted, state)
    };
    let Some(player) = scheduler.player(id) else {
        ack(scheduler, id, sequence);
        return;
    };
    let eye = (
        player.position().x,
        player.position().y + EYE_HEIGHT,
        player.position().z,
    );
    let reach = if player.game_mode() == CREATIVE {
        CREATIVE_REACH
    } else {
        SURVIVAL_REACH
    };
    let hit_point = (
        f64::from(position.x) + f64::from(cursor.0),
        f64::from(position.y) + f64::from(cursor.1),
        f64::from(position.z) + f64::from(cursor.2),
    );
    if distance(eye, hit_point) > reach {
        debug!(
            player = id,
            "placement refused: the hit point is beyond the block reach"
        );
        refuse(
            scheduler,
            id,
            position,
            hit_state,
            target,
            target_state,
            sequence,
        );
        return;
    }
    if occupied(scheduler, target) {
        // The S24b own-cell case: the client predicts the block, vanilla
        // refuses and corrects both cells.
        debug!(
            player = id,
            "placement refused: the target cell intersects a player"
        );
        refuse(
            scheduler,
            id,
            position,
            hit_state,
            target,
            target_state,
            sequence,
        );
        return;
    }
    if !is_replaceable(blocks, target_state) {
        // A non-replaceable target is not a placement at all (the client
        // would show the block's own use); only the ack is certain.
        ack(scheduler, id, sequence);
        return;
    }
    let held = if hand == 1 {
        player.inventory().stack(45)
    } else {
        Some(player.inventory().held())
    };
    let Some(held) = held else {
        ack(scheduler, id, sequence);
        return;
    };
    if held.is_empty() {
        ack(scheduler, id, sequence);
        return;
    }
    let Some(item_name) = items.entry_name(ITEM_REGISTRY, held.item) else {
        ack(scheduler, id, sequence);
        return;
    };
    let lookup = |cell: Position| store.block(cell.x, cell.y, cell.z);
    let Some(new_state) = placed_state(
        blocks,
        &lookup,
        item_name,
        face,
        cursor.1,
        player.position().yaw,
        target,
        target_state,
    ) else {
        ack(scheduler, id, sequence);
        return;
    };
    match store.set_block(target.x, target.y, target.z, new_state) {
        Ok(_) => {
            // The S24b answer order: the hit cell, the cell the client's
            // prediction sized up (the clicked face's neighbour), then the
            // actual destination, then the ack. When the destination is the
            // clicked face's neighbour the last two are the same cell and
            // carry the same state, which is the traced double update.
            let hit_after = if target == position {
                new_state
            } else {
                hit_state
            };
            let predicted_after = if predicted == target {
                new_state
            } else {
                store
                    .block(predicted.x, predicted.y, predicted.z)
                    .unwrap_or(hit_state)
            };
            announce(
                scheduler,
                id,
                position,
                hit_after,
                predicted,
                predicted_after,
                target,
                new_state,
                sequence,
            );
        }
        Err(err) => {
            warn!(
                x = target.x,
                y = target.y,
                z = target.z,
                %err,
                "placement write failed, the cell is left unchanged"
            );
            ack(scheduler, id, sequence);
        }
    }
}

/// The cell a click on `position` with `face` places into.
fn neighbour(position: Position, face: i32) -> Option<Position> {
    let (dx, dy, dz) = match face {
        0 => (0, -1, 0),
        1 => (0, 1, 0),
        2 => (0, 0, -1),
        3 => (0, 0, 1),
        4 => (-1, 0, 0),
        5 => (1, 0, 0),
        _ => return None,
    };
    Some(Position {
        x: position.x + dx,
        y: position.y + dy,
        z: position.z + dz,
    })
}

/// Whether a state can be replaced by a placed block.
fn is_replaceable(blocks: &BlockStates, state: i32) -> bool {
    blocks.is_air(state)
        || blocks
            .block_name(state)
            .is_some_and(|name| REPLACEABLE.contains(&name))
}

/// The state a placement leaves in `target`, or `None` when the report cannot
/// describe it.
#[allow(
    clippy::too_many_arguments,
    reason = "the placement inputs are all trace fields"
)]
fn placed_state(
    blocks: &BlockStates,
    lookup: &dyn Fn(Position) -> Option<i32>,
    name: &str,
    face: i32,
    cursor_y: f32,
    yaw: f32,
    target: Position,
    target_state: i32,
) -> Option<i32> {
    let default = blocks.default_id(name)?;
    let properties = blocks.properties(default).cloned().unwrap_or_default();
    let mut overrides = Properties::new();
    if properties.contains_key("axis") {
        overrides.insert("axis".to_owned(), axis_of_face(face).to_owned());
    }
    if name.ends_with("_slab") && properties.contains_key("type") {
        overrides.insert("type".to_owned(), slab_type(cursor_y).to_owned());
    }
    if name.ends_with("_stairs") {
        let facing = facing_of_yaw(yaw);
        let half = if cursor_y < 0.5 { "bottom" } else { "top" };
        overrides.insert("facing".to_owned(), facing.to_owned());
        overrides.insert("half".to_owned(), half.to_owned());
        overrides.insert(
            "shape".to_owned(),
            stair_shape(blocks, lookup, target, facing, half).to_owned(),
        );
    }
    if properties.contains_key("waterlogged") && blocks.is_water(target_state) {
        overrides.insert("waterlogged".to_owned(), "true".to_owned());
    }
    blocks.state_id_with_overrides(name, &overrides)
}

/// The `axis` a log-like block takes from the clicked face.
fn axis_of_face(face: i32) -> &'static str {
    match face {
        0 | 1 => "y",
        2 | 3 => "z",
        _ => "x",
    }
}

/// The `type` of a slab from the cursor height inside the clicked block.
fn slab_type(cursor_y: f32) -> &'static str {
    if cursor_y < 0.5 {
        "bottom"
    } else {
        "top"
    }
}

/// The horizontal `facing` of a placed stair from the player's yaw.
///
/// Vanilla yaw: `0` south (`+z`), `90` west (`-x`), `180` north (`-z`),
/// `270` east (`+x`). The S17 trace places a south-facing stair at yaw `0`
/// and a west-facing one at yaw `90`.
fn facing_of_yaw(yaw: f32) -> &'static str {
    let turned = yaw.rem_euclid(360.0);
    let quarter = ((turned + 45.0) / 90.0).floor() as i32 % 4;
    match quarter {
        0 => "south",
        1 => "west",
        2 => "north",
        _ => "east",
    }
}

/// The `shape` of a placed stair from its neighbours.
///
/// SOURCE: the S24b table (`docs/BLOCK-PLACING.md` §3): a lone stair, a stair
/// with a perpendicular neighbour in front **or** behind, are all `straight`;
/// only a stair with both a front and a behind neighbour facing the same
/// perpendicular direction moves off `straight`, and the measured case is
/// `outer_left` (front and behind face the placed stair's counter-clockwise
/// side). Everything else stays `straight` — vanilla's `inner_*` and
/// `outer_right` cases are not measured and are recorded as debt.
fn stair_shape(
    blocks: &BlockStates,
    lookup: &dyn Fn(Position) -> Option<i32>,
    target: Position,
    facing: &str,
    half: &str,
) -> &'static str {
    let Some(front) = shifted(target, facing, 1) else {
        return "straight";
    };
    let Some(behind) = shifted(target, facing, -1) else {
        return "straight";
    };
    let Some(front_state) = lookup(front) else {
        return "straight";
    };
    let Some(behind_state) = lookup(behind) else {
        return "straight";
    };
    let clockwise = counter_clockwise(facing);
    if is_stairs_towards(blocks, front_state, half, clockwise)
        && is_stairs_towards(blocks, behind_state, half, clockwise)
    {
        "outer_left"
    } else {
        "straight"
    }
}

/// The cell `distance` steps from `position` along a horizontal `facing`.
fn shifted(position: Position, facing: &str, distance: i32) -> Option<Position> {
    let (dx, dz) = match facing {
        "north" => (0, -1),
        "south" => (0, 1),
        "west" => (-1, 0),
        "east" => (1, 0),
        _ => return None,
    };
    Some(Position {
        x: position.x + dx * distance,
        z: position.z + dz * distance,
        y: position.y,
    })
}

/// The horizontal direction 90° counter-clockwise from `facing`.
fn counter_clockwise(facing: &str) -> &'static str {
    match facing {
        "north" => "west",
        "west" => "south",
        "south" => "east",
        _ => "north",
    }
}

/// Whether `state` is a stair of the same half facing `facing`.
fn is_stairs_towards(blocks: &BlockStates, state: i32, half: &str, facing: &str) -> bool {
    let Some(name) = blocks.block_name(state) else {
        return false;
    };
    if !name.ends_with("_stairs") {
        return false;
    }
    let Some(properties) = blocks.properties(state) else {
        return false;
    };
    properties.get("half").is_some_and(|value| value == half)
        && properties
            .get("facing")
            .is_some_and(|value| value == facing)
}

/// Euclidean distance between two points.
fn distance(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let dx = a.0 - b.0;
    let dy = a.1 - b.1;
    let dz = a.2 - b.2;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Whether any online player's box intersects the block at `target`.
///
/// SOURCE: the S24b own-cell case (the player's own box). The vanilla server
/// also refuses a cell another entity occupies; the S17 trace never sent that
/// request (the client aimed at the other player instead), so checking every
/// player is the conservative reading of the same rule.
fn occupied(scheduler: &Scheduler, target: Position) -> bool {
    scheduler.players().any(|player| {
        let position = player.position();
        let min_x = f64::from(target.x);
        let max_x = min_x + 1.0;
        let min_y = f64::from(target.y);
        let max_y = min_y + 1.0;
        let min_z = f64::from(target.z);
        let max_z = min_z + 1.0;
        position.x + PLAYER_HALF_WIDTH > min_x
            && position.x - PLAYER_HALF_WIDTH < max_x
            && position.y + PLAYER_HEIGHT > min_y
            && position.y < max_y
            && position.z + PLAYER_HALF_WIDTH > min_z
            && position.z - PLAYER_HALF_WIDTH < max_z
    })
}

/// A successful placement: the S24b per-connection answer order.
///
/// The actor gets the hit cell, the cell its prediction marked (the clicked
/// face's neighbour), the actual destination, and the ack. For a target behind
/// the face the middle and last updates are the same cell and state (the S24b
/// double update); when the clicked replaceable cell is replaced in place they
/// differ (S17 `replace-short-grass`).
#[allow(
    clippy::too_many_arguments,
    reason = "the placement answer is one linear send"
)]
fn announce(
    scheduler: &mut Scheduler,
    actor: PlayerId,
    hit: Position,
    hit_state: i32,
    predicted: Position,
    predicted_state: i32,
    target: Position,
    new_state: i32,
    sequence: i32,
) {
    deliver(
        scheduler,
        actor,
        ServerMessage::BlockUpdate {
            position: hit,
            state: hit_state,
        },
    );
    deliver(
        scheduler,
        actor,
        ServerMessage::BlockUpdate {
            position: predicted,
            state: predicted_state,
        },
    );
    deliver(
        scheduler,
        actor,
        ServerMessage::BlockUpdate {
            position: target,
            state: new_state,
        },
    );
    let others: Vec<PlayerId> = scheduler
        .regions()
        .iter()
        .flat_map(|region| region.player_ids())
        .filter(|id| *id != actor)
        .collect();
    for id in others {
        deliver(
            scheduler,
            id,
            ServerMessage::BlockUpdate {
                position: target,
                state: new_state,
            },
        );
    }
    ack(scheduler, actor, sequence);
}

/// A refused placement: correct the clicked cell and the untouched target for
/// the actor, acknowledge, and tell the observers nothing (S24b own-cell).
fn refuse(
    scheduler: &mut Scheduler,
    actor: PlayerId,
    hit: Position,
    hit_state: i32,
    target: Position,
    target_state: i32,
    sequence: i32,
) {
    deliver(
        scheduler,
        actor,
        ServerMessage::BlockUpdate {
            position: hit,
            state: hit_state,
        },
    );
    deliver(
        scheduler,
        actor,
        ServerMessage::BlockUpdate {
            position: target,
            state: target_state,
        },
    );
    ack(scheduler, actor, sequence);
}

/// Sends one `block_changed_ack` to the actor.
fn ack(scheduler: &mut Scheduler, id: PlayerId, sequence: i32) {
    deliver(scheduler, id, ServerMessage::BlockChangedAck { sequence });
}

#[cfg(test)]
mod tests {
    use super::*;

    const OAK_LOG: &str = "minecraft:oak_log";
    const OAK_SLAB: &str = "minecraft:oak_slab";
    const OAK_STAIRS: &str = "minecraft:oak_stairs";

    /// A report slice the placement tests can resolve.
    pub(super) fn blocks() -> BlockStates {
        BlockStates::parse(
            r#"{
              "minecraft:air": {"states": [{"id": 0, "default": true}]},
              "minecraft:stone": {"states": [{"id": 1, "default": true}]},
              "minecraft:water": {"states": [{"id": 89, "default": true}]},
              "minecraft:short_grass": {"states": [{"id": 98, "default": true}]},
              "minecraft:snow": {"states": [{"id": 4000, "default": true}]},
              "minecraft:oak_log": {"states": [
                {"id": 141, "default": true, "properties": {"axis": "y"}},
                {"id": 142, "properties": {"axis": "x"}},
                {"id": 143, "properties": {"axis": "z"}}
              ]},
              "minecraft:oak_slab": {"states": [
                {"id": 15180, "default": true, "properties": {"type": "bottom", "waterlogged": "false"}},
                {"id": 15181, "properties": {"type": "top", "waterlogged": "false"}},
                {"id": 15182, "properties": {"type": "bottom", "waterlogged": "true"}},
                {"id": 15183, "properties": {"type": "top", "waterlogged": "true"}}
              ]},
              "minecraft:oak_stairs": {"states": [
                {"id": 5494, "default": true, "properties": {"facing": "south", "half": "bottom", "shape": "straight", "waterlogged": "false"}},
                {"id": 5495, "properties": {"facing": "south", "half": "bottom", "shape": "outer_left", "waterlogged": "false"}},
                {"id": 5496, "properties": {"facing": "south", "half": "top", "shape": "straight", "waterlogged": "false"}},
                {"id": 5514, "properties": {"facing": "west", "half": "bottom", "shape": "straight", "waterlogged": "false"}},
                {"id": 5515, "properties": {"facing": "west", "half": "bottom", "shape": "outer_left", "waterlogged": "false"}},
                {"id": 5516, "properties": {"facing": "west", "half": "bottom", "shape": "outer_right", "waterlogged": "false"}},
                {"id": 5517, "properties": {"facing": "west", "half": "top", "shape": "straight", "waterlogged": "false"}},
                {"id": 5518, "properties": {"facing": "north", "half": "bottom", "shape": "straight", "waterlogged": "false"}},
                {"id": 5519, "properties": {"facing": "east", "half": "bottom", "shape": "straight", "waterlogged": "false"}},
                {"id": 5520, "properties": {"facing": "east", "half": "bottom", "shape": "outer_left", "waterlogged": "false"}}
              ]}
            }"#,
            "placement fixture".to_owned(),
        )
        .expect("valid report")
    }

    fn position(x: i32, y: i32, z: i32) -> Position {
        Position { x, y, z }
    }

    /// No neighbour cells: every stair shape lookup misses.
    fn no_neighbours() -> impl Fn(Position) -> Option<i32> {
        |_| None
    }

    /// The S17 `log-face` case: face north leaves `axis=z`.
    #[test]
    fn a_log_takes_its_axis_from_the_face() {
        let blocks = blocks();
        let neighbours = no_neighbours();
        let state = placed_state(
            &blocks,
            &neighbours,
            OAK_LOG,
            2,
            0.46,
            0.0,
            position(0, 0, 0),
            1,
        )
        .expect("a log state");
        assert_eq!(blocks.block_name(state), Some(OAK_LOG));
        assert_eq!(
            blocks.properties(state).and_then(|map| map.get("axis")),
            Some(&"z".to_owned())
        );
    }

    /// The S17 `slab-low-click` case: a cursor under the middle is `bottom`.
    #[test]
    fn a_slab_takes_its_type_from_the_cursor() {
        let blocks = blocks();
        let neighbours = no_neighbours();
        let low = placed_state(
            &blocks,
            &neighbours,
            OAK_SLAB,
            2,
            0.2264,
            0.0,
            position(0, 0, 0),
            1,
        )
        .expect("a slab state");
        assert_eq!(low, 15180);
        let high = placed_state(
            &blocks,
            &neighbours,
            OAK_SLAB,
            2,
            0.8,
            0.0,
            position(0, 0, 0),
            1,
        )
        .expect("a slab state");
        assert_eq!(high, 15181);
    }

    /// The S17 `stairs-south-view` and `stairs-west-view` cases: yaw picks the
    /// facing, the cursor the half.
    #[test]
    fn stairs_take_facing_from_yaw_and_half_from_the_cursor() {
        let blocks = blocks();
        let neighbours = no_neighbours();
        let south = placed_state(
            &blocks,
            &neighbours,
            OAK_STAIRS,
            2,
            0.46,
            0.0,
            position(0, 0, 0),
            1,
        )
        .expect("a stair state");
        assert_eq!(south, 5494);
        let west = placed_state(
            &blocks,
            &neighbours,
            OAK_STAIRS,
            5,
            0.5,
            90.0,
            position(0, 0, 0),
            1,
        )
        .expect("a stair state");
        assert_eq!(west, 5517, "the cursor at 0.5 is the top half");
        let west_low = placed_state(
            &blocks,
            &neighbours,
            OAK_STAIRS,
            5,
            0.4,
            90.0,
            position(0, 0, 0),
            1,
        )
        .expect("a stair state");
        assert_eq!(west_low, 5514);
    }

    /// The S24b waterlogged stair: a stair placed into water becomes
    /// `waterlogged=true` (state 5473 in the real report).
    #[test]
    fn a_stair_placed_into_water_is_waterlogged() {
        let blocks = blocks();
        let neighbours = no_neighbours();
        // The target state is passed in: the trace placed the block into a
        // water source cell.
        let state = placed_state(
            &blocks,
            &neighbours,
            OAK_SLAB,
            1,
            0.4,
            180.0,
            position(0, 0, 0),
            89,
        )
        .expect("a slab state");
        assert_eq!(
            blocks
                .properties(state)
                .and_then(|map| map.get("waterlogged")),
            Some(&"true".to_owned())
        );
    }

    /// The one measured off-`straight` shape: a stair with a same-half stair in
    /// front and behind, both facing the counter-clockwise side, is
    /// `outer_left` (S24b `front_north_behind_north`).
    #[test]
    fn a_front_and_behind_neighbour_facing_one_way_is_outer_left() {
        let blocks = blocks();
        let neighbours = |cell: Position| {
            if cell == position(1, 0, 0) || cell == position(-1, 0, 0) {
                // `minecraft:oak_stairs[facing=north, half=bottom]`.
                Some(5518)
            } else {
                None
            }
        };
        let state = placed_state(
            &blocks,
            &neighbours,
            OAK_STAIRS,
            2,
            0.4,
            270.0,
            position(0, 0, 0),
            1,
        )
        .expect("a stair state");
        assert_eq!(state, 5520);
    }

    /// The reach boundary of §24.0b: the S24b hit point at 4.5 is inside the
    /// creative range, six blocks are outside.
    #[test]
    fn the_creative_reach_matches_the_s24b_sweep() {
        let eye = (86.0, 202.62, -39.5);
        assert!(distance(eye, (81.0, 202.62, -39.5)) <= CREATIVE_REACH);
        assert!(distance((106.0, 202.62, -39.5), (101.5, 202.62, -39.5)) < CREATIVE_REACH);
        assert!(distance((147.0, 202.62, -39.5), (141.5, 202.62, -39.5)) > CREATIVE_REACH);
        assert!(distance(eye, (81.0, 202.62, -39.5)) > SURVIVAL_REACH);
    }

    #[test]
    fn the_target_cell_is_the_neighbour_behind_the_face() {
        assert_eq!(
            neighbour(position(80, 202, -40), 5),
            Some(position(81, 202, -40))
        );
        assert_eq!(
            neighbour(position(170, 201, -40), 1),
            Some(position(170, 202, -40))
        );
        assert_eq!(neighbour(position(0, 0, 0), 2), Some(position(0, 0, -1)));
        assert_eq!(neighbour(position(0, 0, 0), 6), None);
    }

    #[test]
    fn air_water_grass_and_snow_are_replaceable() {
        let blocks = blocks();
        assert!(is_replaceable(&blocks, 0), "air");
        assert!(is_replaceable(&blocks, 89), "water");
        assert!(is_replaceable(&blocks, 98), "short grass");
        assert!(is_replaceable(&blocks, 4000), "snow");
        assert!(!is_replaceable(&blocks, 1), "stone");
        assert!(!is_replaceable(&blocks, 141), "log");
    }

    #[test]
    fn a_facing_follows_the_vanilla_yaw_quarters() {
        assert_eq!(facing_of_yaw(0.0), "south");
        assert_eq!(facing_of_yaw(90.0), "west");
        assert_eq!(facing_of_yaw(180.0), "north");
        assert_eq!(facing_of_yaw(-90.0), "east");
        assert_eq!(facing_of_yaw(359.0), "south");
    }
}

/// Tick-path tests of a placement: the store write and the per-connection
/// answer order of the S24b trace (Шаг 24b).
#[cfg(test)]
mod trace_tests {
    use std::sync::Arc;

    use protocol::{
        ChunkSection, HeightmapRecord, ItemStack, LevelChunkWithLight, LightData, PaletteContainer,
    };
    use tokio::sync::mpsc;

    use crate::chunks::ReadyChunks;
    use crate::game::test_support::join_at_mode;
    use crate::game::{Region, Replay, Scheduler, ServerMessage, SINGLE_REGION};
    use crate::store::ChunkStore;

    use super::{place, BlockPlacement, CREATIVE};

    const STONE: i32 = 1;
    /// `minecraft:short_grass` in the placement fixture.
    const SHORT_GRASS: i32 = 98;

    /// The registry slice with the item the tests hold.
    fn items() -> data::RegistryIds {
        data::registries_from_json(
            r#"{"minecraft:item": {"entries": {
                "minecraft:stone": {"protocol_id": 1}
            }}}"#,
        )
        .expect("valid registries report")
    }

    /// A chunk whose every cell is air, so a write is the only non-air state.
    fn air_chunk(chunk_x: i32, chunk_z: i32) -> LevelChunkWithLight {
        LevelChunkWithLight {
            chunk_x,
            chunk_z,
            heightmaps: vec![HeightmapRecord {
                id: 4,
                words: vec![0],
            }],
            sections: (0..24)
                .map(|_| ChunkSection {
                    non_air: 0,
                    fluid: 0,
                    blocks: PaletteContainer::Single(0),
                    biomes: PaletteContainer::Single(1),
                })
                .collect(),
            block_entities: Vec::new(),
            light: LightData {
                sky_mask: vec![0x00],
                block_mask: vec![0x00],
                empty_sky_mask: vec![0x01],
                empty_block_mask: vec![0x01],
                sky_arrays: Vec::new(),
                block_arrays: Vec::new(),
            },
        }
    }

    /// A scheduler with a creative actor and a survival observer, both in one
    /// region, plus their outbound queues.
    fn players(
        actor_at: (i32, i32, i32),
    ) -> (
        Scheduler,
        mpsc::Receiver<ServerMessage>,
        mpsc::Receiver<ServerMessage>,
    ) {
        let mut scheduler = Scheduler::new(vec![Region::new(SINGLE_REGION)]);
        let (actor, _slot, actor_rx) = join_at_mode(1, actor_at, CREATIVE);
        let (observer, _slot, observer_rx) = join_at_mode(2, (8, 64, 8), 0);
        scheduler.insert_player(actor);
        scheduler.insert_player(observer);
        // The actor holds one stone in the first hotbar slot.
        let inventory = scheduler
            .player_mut(1)
            .expect("the actor joined")
            .inventory_mut();
        inventory.set_creative(36, ItemStack::new(STONE, 1));
        (scheduler, actor_rx, observer_rx)
    }

    /// Every message queued so far.
    fn drain(rx: &mut mpsc::Receiver<ServerMessage>) -> Vec<ServerMessage> {
        let mut messages = Vec::new();
        while let Ok(message) = rx.try_recv() {
            messages.push(message);
        }
        messages
    }

    /// The S24b per-connection order: the actor sees the hit cell, the target
    /// twice and the ack; the observer sees the target once.
    #[test]
    fn a_placement_writes_the_cell_and_answers_in_the_traced_order() {
        let blocks = Arc::new(super::tests::blocks());
        let store = Arc::new(ChunkStore::new(
            ReadyChunks::new(vec![air_chunk(0, 0)]),
            Arc::clone(&blocks),
        ));
        store.set_block(0, 64, 0, STONE).expect("hit cell");
        let items = items();
        let (mut scheduler, mut actor_rx, mut observer_rx) = players((0, 64, 3));
        let mut replay = Replay::<Vec<u8>>::new(None);

        place(
            &mut scheduler,
            &mut replay,
            1,
            BlockPlacement {
                id: 1,
                hand: 0,
                position: protocol::Position { x: 0, y: 64, z: 0 },
                face: 1,
                cursor: (0.5, 1.0, 0.5),
                inside_block: false,
                world_border: false,
                sequence: 7,
            },
            Some(&store),
            Some(&blocks),
            Some(&items),
        );

        assert_eq!(
            store.block(0, 65, 0),
            Some(STONE),
            "the target cell was written"
        );
        let actor = drain(&mut actor_rx);
        assert!(
            matches!(
                actor.as_slice(),
                [
                    ServerMessage::BlockUpdate { position: hit, state: STONE },
                    ServerMessage::BlockUpdate { position: target, state: STONE },
                    ServerMessage::BlockUpdate { position: again, state: STONE },
                    ServerMessage::BlockChangedAck { sequence: 7 },
                ] if *hit == protocol::Position { x: 0, y: 64, z: 0 }
                    && *target == protocol::Position { x: 0, y: 65, z: 0 }
                    && *again == *target
            ),
            "the actor stream: {actor:?}"
        );
        let observer = drain(&mut observer_rx);
        assert_eq!(
            observer.len(),
            1,
            "the observer sees the target once: {observer:?}"
        );
        assert!(matches!(
            observer[0],
            ServerMessage::BlockUpdate { position, state: STONE }
                if position == protocol::Position { x: 0, y: 65, z: 0 }
        ));
    }

    /// The S17 `replace-short-grass` click: the crosshair stops on the grass
    /// outline, so the packet names the grass cell itself with a north face. The
    /// stone replaces that cell (not the cell behind the face) and the actor's
    /// three updates name the grass cell, the cell behind the face and the grass
    /// cell again (2026-11-18_s17-vanilla-step23-r2, A seq 80534/80536-80538).
    #[test]
    fn a_click_on_replaceable_grass_replaces_the_clicked_cell() {
        let blocks = Arc::new(super::tests::blocks());
        let store = Arc::new(ChunkStore::new(
            ReadyChunks::new(vec![air_chunk(0, 0), air_chunk(0, -1)]),
            Arc::clone(&blocks),
        ));
        store.set_block(0, 64, 0, SHORT_GRASS).expect("grass cell");
        let items = items();
        let (mut scheduler, mut actor_rx, mut observer_rx) = players((0, 64, 3));
        let mut replay = Replay::<Vec<u8>>::new(None);

        place(
            &mut scheduler,
            &mut replay,
            1,
            BlockPlacement {
                id: 1,
                hand: 0,
                position: protocol::Position { x: 0, y: 64, z: 0 },
                face: 2,
                cursor: (0.5, 0.4934, 0.125),
                inside_block: false,
                world_border: false,
                sequence: 5,
            },
            Some(&store),
            Some(&blocks),
            Some(&items),
        );

        assert_eq!(store.block(0, 64, 0), Some(STONE), "the grass was replaced");
        assert_eq!(store.block(0, 64, -1), Some(0), "the cell behind stays air");
        let actor = drain(&mut actor_rx);
        assert!(
            matches!(
                actor.as_slice(),
                [
                    ServerMessage::BlockUpdate { position: first, state: STONE },
                    ServerMessage::BlockUpdate { position: behind, state: 0 },
                    ServerMessage::BlockUpdate { position: again, state: STONE },
                    ServerMessage::BlockChangedAck { sequence: 5 },
                ] if *first == protocol::Position { x: 0, y: 64, z: 0 }
                    && *behind == protocol::Position { x: 0, y: 64, z: -1 }
                    && *again == *first
            ),
            "the grass answer stream: {actor:?}"
        );
        let observer = drain(&mut observer_rx);
        assert!(
            matches!(
                observer.as_slice(),
                [ServerMessage::BlockUpdate { position, state: STONE }]
                    if *position == protocol::Position { x: 0, y: 64, z: 0 }
            ),
            "the observer sees the grass cell once: {observer:?}"
        );
    }

    /// The S24b own-cell refusal: both cells are corrected for the actor, the
    /// observer hears nothing and the target stays air.
    #[test]
    fn a_placement_into_the_actors_own_cell_is_refused() {
        let blocks = Arc::new(super::tests::blocks());
        let store = Arc::new(ChunkStore::new(
            ReadyChunks::new(vec![air_chunk(0, 0)]),
            Arc::clone(&blocks),
        ));
        store.set_block(0, 64, 0, STONE).expect("hit cell");
        let items = items();
        // The actor stands in the cell above the hit block: the target cell
        // of an up-face click is exactly the player's own box.
        let (mut scheduler, mut actor_rx, mut observer_rx) = players((0, 65, 0));
        let mut replay = Replay::<Vec<u8>>::new(None);

        place(
            &mut scheduler,
            &mut replay,
            1,
            BlockPlacement {
                id: 1,
                hand: 0,
                position: protocol::Position { x: 0, y: 64, z: 0 },
                face: 1,
                cursor: (0.5, 1.0, 0.5),
                inside_block: true,
                world_border: false,
                sequence: 8,
            },
            Some(&store),
            Some(&blocks),
            Some(&items),
        );

        assert_eq!(store.block(0, 65, 0), Some(0), "the target stays air");
        let actor = drain(&mut actor_rx);
        assert!(
            matches!(
                actor.as_slice(),
                [
                    ServerMessage::BlockUpdate { state: STONE, .. },
                    ServerMessage::BlockUpdate { state: 0, .. },
                    ServerMessage::BlockChangedAck { sequence: 8 },
                ]
            ),
            "the refusal stream: {actor:?}"
        );
        assert!(drain(&mut observer_rx).is_empty(), "observers hear nothing");
    }

    /// A click beyond the creative reach is refused the same way (the server
    /// answer to the case the client already drops; hypothesis, §24.0b).
    #[test]
    fn a_click_beyond_the_reach_is_refused() {
        let blocks = Arc::new(super::tests::blocks());
        let store = Arc::new(ChunkStore::new(
            ReadyChunks::new(vec![air_chunk(0, 0)]),
            Arc::clone(&blocks),
        ));
        store.set_block(0, 64, 0, STONE).expect("hit cell");
        let items = items();
        let (mut scheduler, mut actor_rx, _observer_rx) = players((0, 64, 9));
        let mut replay = Replay::<Vec<u8>>::new(None);

        place(
            &mut scheduler,
            &mut replay,
            1,
            BlockPlacement {
                id: 1,
                hand: 0,
                position: protocol::Position { x: 0, y: 64, z: 0 },
                face: 1,
                cursor: (0.5, 1.0, 0.5),
                inside_block: false,
                world_border: false,
                sequence: 9,
            },
            Some(&store),
            Some(&blocks),
            Some(&items),
        );

        assert_eq!(store.block(0, 65, 0), Some(0), "the target stays air");
        assert!(matches!(
            drain(&mut actor_rx).as_slice(),
            [
                ServerMessage::BlockUpdate { state: STONE, .. },
                ServerMessage::BlockUpdate { state: 0, .. },
                ServerMessage::BlockChangedAck { sequence: 9 },
            ]
        ));
    }
}
