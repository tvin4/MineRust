//! Shared helpers of the M4.5 Шаг 4 (в)/(г) parity tests: reading the JSONL
//! traces written by the two-bot driver, filtering the clientbound packets
//! about bot B, aligning positions in time, and starting our own in-process
//! v3 server with real sockets (the `window_walk_ours` pattern).
//!
//! Used from `tests/entity_stream_parity.rs` (в) and
//! `tests/entity_view_distance.rs` (г); each of those declares `mod twobot`
//! and `mod scenarios` at its own crate root, which this module reaches
//! through `crate::twobot` / `crate::scenarios`.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use parity::{from_base64, ParityEnv};
use protocol::{AddEntity, PacketReader, PlayerInfoRemove, PlayerInfoUpdate, Uuid};

use crate::scenarios::{BScript, Observer, Scene};
use crate::twobot::{run_bot_at, BotReport, Shared, BOT_A, BOT_B};

/// Where the traces and the reports land (outside the repository).
pub const TRACE_DIR: &str = r"C:\MineRust-data\traces";
/// The frozen v3 world, read-only for our in-process server.
pub const V3_TEMPLATE: &str = r"C:\MineRust-data\world-template-v3";
/// Budget for a bot to reach play before the scenario gives up.
pub const JOIN_WAIT: Duration = Duration::from_secs(180);
/// Per-bot play budget: the script plus a margin.
pub const SAFETY: Duration = Duration::from_secs(240);

/// One packet of a JSONL trace, decoded back to bytes.
#[derive(Clone)]
pub struct Line {
    /// Packet name from the packets report, when resolvable.
    pub name: Option<String>,
    /// `dir` of the line (`clientbound` / `serverbound`).
    pub dir: String,
    /// Wire time in milliseconds, as the JSONL recorded it.
    pub t_ms: f64,
    /// The payload bytes (base64-decoded).
    pub bytes: Vec<u8>,
}

/// Reads one JSONL trace. Panics on a malformed line: a broken trace is a
/// reading failure, not a scenario to guess around.
pub fn load_lines(path: &Path) -> Vec<Line> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    let mut out = Vec::new();
    for raw in text.lines() {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        let value: serde_json::Value =
            serde_json::from_str(raw).unwrap_or_else(|error| panic!("bad jsonl line: {error}"));
        let seq = value["seq"].as_u64().unwrap_or(0);
        let name = value
            .get("name")
            .and_then(|n| n.as_str())
            .map(str::to_owned);
        let bytes = from_base64(value["b64"].as_str().unwrap_or_default())
            .unwrap_or_else(|| panic!("bad base64 in line {seq}"));
        out.push(Line {
            name,
            dir: value["dir"].as_str().unwrap_or_default().to_owned(),
            t_ms: value["t_ms"].as_f64().unwrap_or(0.0),
            bytes,
        });
    }
    out
}

/// The entity id a viewer assigned to the player with `uuid`: the id of the
/// `add_entity` that carries that UUID. `None` when the player never spawned
/// for this viewer (a fact, not a default).
pub fn entity_id_by_uuid(lines: &[Line], uuid: Uuid) -> Option<i32> {
    for line in lines {
        if line.name.as_deref() != Some("minecraft:add_entity") {
            continue;
        }
        if let Ok(entity) = AddEntity::decode_payload(&mut PacketReader::new(&line.bytes)) {
            if entity.uuid == uuid {
                return Some(entity.entity_id);
            }
        }
    }
    None
}

/// The entity id B owns in a trace (see [`entity_id_by_uuid`]).
pub fn b_entity_id(lines: &[Line], b_uuid: Uuid) -> Option<i32> {
    entity_id_by_uuid(lines, b_uuid)
}

/// The leading VarInt of a payload (an entity id), when it decodes.
pub fn leading_id(bytes: &[u8]) -> Option<i32> {
    protocol::decode_varint(bytes).ok().map(|(value, _)| value)
}

/// Entity id a packet is about, per the observed layout (leading VarInt, with
/// the `entity_event` event byte before it). Returns `None` for
/// `remove_entities` (a list) and the `player_info_*` packets (no id); those
/// are handled by the caller.
pub fn entity_id_of(name: &str, bytes: &[u8]) -> Option<i32> {
    match name {
        "minecraft:add_entity" => AddEntity::decode_payload(&mut PacketReader::new(bytes))
            .ok()
            .map(|entity| entity.entity_id),
        "minecraft:entity_event" => bytes.get(1..).and_then(leading_id),
        "minecraft:move_entity_pos"
        | "minecraft:move_entity_pos_rot"
        | "minecraft:move_entity_rot"
        | "minecraft:rotate_head"
        | "minecraft:entity_position_sync"
        | "minecraft:set_entity_data"
        | "minecraft:set_entity_motion"
        | "minecraft:update_attributes"
        | "minecraft:set_equipment"
        | "minecraft:teleport_entity"
        | "minecraft:set_passengers" => leading_id(bytes),
        _ => None,
    }
}

/// Ids of a `remove_entities` list (VarInt count, then that many ids).
pub fn remove_ids(bytes: &[u8]) -> Vec<i32> {
    let Ok((count, mut at)) = protocol::decode_varint(bytes) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for _ in 0..count.clamp(0, 4096) {
        match protocol::decode_varint(&bytes[at..]) {
            Ok((value, used)) => {
                out.push(value);
                at += used;
            }
            Err(_) => break,
        }
    }
    out
}

/// Every clientbound packet of `lines` that is "about B": reference B's
/// entity id, or name B's UUID in a `player_info_*` packet.
pub fn about_b(lines: &[Line], b_id: i32, b_uuid: Uuid) -> Vec<Line> {
    let mut out = Vec::new();
    for line in lines {
        if line.dir != "clientbound" {
            continue;
        }
        let Some(name) = line.name.as_deref() else {
            continue;
        };
        let about = match name {
            "minecraft:player_info_update" => {
                PlayerInfoUpdate::decode_payload(&mut PacketReader::new(&line.bytes))
                    .map(|update| update.players.iter().any(|entry| entry.uuid == b_uuid))
                    .unwrap_or(false)
            }
            "minecraft:player_info_remove" => {
                PlayerInfoRemove::decode_payload(&mut PacketReader::new(&line.bytes))
                    .map(|remove| remove.uuids.contains(&b_uuid))
                    .unwrap_or(false)
            }
            "minecraft:remove_entities" => remove_ids(&line.bytes).contains(&b_id),
            _ => entity_id_of(name, &line.bytes) == Some(b_id),
        };
        if about {
            out.push(line.clone());
        }
    }
    out
}

/// Horizontal `(x, z)` of a serverbound move payload (the walk layout: three
/// `f64` then the angles), when it has enough bytes.
pub fn move_xz(bytes: &[u8]) -> Option<(f64, f64)> {
    let x = bytes.get(0..8)?;
    let z = bytes.get(16..24)?;
    let x = f64::from_be_bytes(x.try_into().ok()?);
    let z = f64::from_be_bytes(z.try_into().ok()?);
    Some((x, z))
}

/// The latest `(x, z)` of a `t_ms`-sorted timeline at or before `t`.
pub fn position_at(timeline: &[(f64, (f64, f64))], t: f64) -> Option<(f64, f64)> {
    let mut found = None;
    for &(stamp, xz) in timeline {
        if stamp <= t {
            found = Some(xz);
        } else {
            break;
        }
    }
    found
}

/// Horizontal distance between two `(x, z)` points.
pub fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dx = a.0 - b.0;
    let dz = a.1 - b.1;
    (dx * dx + dz * dz).sqrt()
}

/// True when a clientbound packet is about the entity `id` (`remove_entities`
/// is a list; the rest carry a leading id).
fn about_id(line: &Line, id: i32) -> bool {
    match line.name.as_deref() {
        Some("minecraft:remove_entities") => remove_ids(&line.bytes).contains(&id),
        Some(name) => entity_id_of(name, &line.bytes) == Some(id),
        None => false,
    }
}

/// A's constant horizontal position, from the single clientbound
/// `player_position` (A stands and never sends a move, so this is its only
/// position record).
pub fn a_position(lines: &[Line]) -> (f64, f64) {
    let teleport = lines.iter().find(|line| {
        line.dir == "clientbound" && line.name.as_deref() == Some("minecraft:player_position")
    });
    let (spawn, _) = teleport
        .and_then(|line| parity::walk::teleport_ack(&line.bytes))
        .expect("A's spawn teleport must be in the trace");
    (spawn[0], spawn[2])
}

/// A bot's serverbound movement timeline: `(t_ms, (x, z))`, time-sorted.
pub fn moves(lines: &[Line]) -> Vec<(f64, (f64, f64))> {
    let mut out: Vec<(f64, (f64, f64))> = lines
        .iter()
        .filter(|line| line.dir == "serverbound")
        .filter(|line| {
            matches!(
                line.name.as_deref(),
                Some("minecraft:move_player_pos") | Some("minecraft:move_player_pos_rot")
            )
        })
        .filter_map(|line| move_xz(&line.bytes).map(|xz| (line.t_ms, xz)))
        .collect();
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// Where a server dropped and re-added B, measured two ways.
///
/// **Geometric** (the truth): B's own clientbound `remove_entities`/`add_entity`
/// about A, with B's position read from B's own serverbound moves. Both
/// records are on B's clock, so no cross-trace bias enters.
///
/// **Naive** (the analyzer's `entities-detail`): A's packets about B, with B's
/// position read from B's move timeline *by raw `t_ms`*. Each bot's `t_ms`
/// starts at its own connection, so this carries the whole clock offset
/// between the two connections (the two real clients in the vanilla trace
/// were ~1.4 s apart; our in-process pair ~0.1 s).
#[derive(Default)]
pub struct Tracking {
    /// Geometric distance at which B's client lost A (out of range).
    pub geom_remove: Option<f64>,
    /// Geometric distance at which B's client got A back.
    pub geom_readd: Option<f64>,
    /// Naive distance of A's `remove_entities` about B.
    pub naive_remove: Option<f64>,
    /// Naive distance of A's `add_entity` about B on the return.
    pub naive_readd: Option<f64>,
    /// Naive maximum distance of any packet about B (the "max visible").
    pub max_visible: Option<f64>,
    /// `t_remove(A) - t_remove(B)`: the measured clock offset, in ms.
    pub offset_ms: Option<f64>,
}

/// Measures one С3 pair (see [`Tracking`]). `a_lines`/`b_lines` are the traces
/// of the two bots; `a_uuid`/`b_uuid` their offline UUIDs.
pub fn tracking(a_lines: &[Line], b_lines: &[Line], a_uuid: Uuid, b_uuid: Uuid) -> Tracking {
    let (Some(a_id_in_b), Some(b_id_in_a)) = (
        entity_id_by_uuid(b_lines, a_uuid),
        entity_id_by_uuid(a_lines, b_uuid),
    ) else {
        return Tracking::default();
    };
    let b_moves = moves(b_lines);
    let a_at = a_position(a_lines);
    // First packet of `name` about `id`, more than 50 blocks away: the
    // out-of-range drop (or the re-add on the way back).
    let first_far = |lines: &[Line], name: &str, id: i32| -> Option<(f64, f64)> {
        lines
            .iter()
            .filter(|line| line.dir == "clientbound" && line.name.as_deref() == Some(name))
            .filter(|line| about_id(line, id))
            .filter_map(|line| {
                position_at(&b_moves, line.t_ms).map(|b| (line.t_ms, distance(a_at, b)))
            })
            .find(|&(_, d)| d > 50.0)
    };
    let geom_remove = first_far(b_lines, "minecraft:remove_entities", a_id_in_b);
    let geom_readd = first_far(b_lines, "minecraft:add_entity", a_id_in_b);
    let naive_remove = first_far(a_lines, "minecraft:remove_entities", b_id_in_a);
    let naive_readd = first_far(a_lines, "minecraft:add_entity", b_id_in_a);
    let max_visible = a_lines
        .iter()
        .filter(|line| line.dir == "clientbound" && about_id(line, b_id_in_a))
        .filter_map(|line| position_at(&b_moves, line.t_ms).map(|b| distance(a_at, b)))
        .fold(None::<f64>, |acc, d| Some(acc.map_or(d, |x| x.max(d))));
    let offset_ms = match (naive_remove, geom_remove) {
        (Some((t_a, _)), Some((t_b, _))) => Some(t_a - t_b),
        _ => None,
    };
    Tracking {
        geom_remove: geom_remove.map(|(_, d)| d),
        geom_readd: geom_readd.map(|(_, d)| d),
        naive_remove: naive_remove.map(|(_, d)| d),
        naive_readd: naive_readd.map(|(_, d)| d),
        max_visible,
        offset_ms,
    }
}

/// Aborts the in-process server on every exit path, including a panic.
pub struct ServerGuard(Option<tokio::task::AbortHandle>);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(abort) = self.0.take() {
            abort.abort();
        }
    }
}

/// Starts our own server in-process on the v3 world at an ephemeral port.
///
/// Mirrors `server::run`: the ready batch is read once **before** the port is
/// used, so the first join does not trigger the whole read inline.
pub async fn start_server(env: &ParityEnv, ids: Arc<PacketIds>) -> (u16, ServerGuard) {
    let mut config = env.config.clone();
    config.world_dir = Some(V3_TEMPLATE.to_owned());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let port = listener.local_addr().expect("local addr").port();
    let status = Arc::new(server::status_info(&config));
    let data = Arc::new(
        data::load_from_jar(
            &config.client_jar,
            config.registries_report.as_deref().map(Path::new),
        )
        .expect("game data from the jar"),
    );
    let (loader, world) = server::build_world(&config, Arc::clone(&data)).expect("v3 world builds");
    if let Some(loader) = &loader {
        let reply = loader.load_batch().await.expect("warm-up queued");
        reply
            .await
            .expect("warm-up reply alive")
            .expect("warm-up read");
    }
    let game_config = net::game::GameConfig {
        workers: config.workers,
        region_side: config.region_side,
        ..net::game::GameConfig::default()
    };
    let server = tokio::spawn(net::serve_with_metrics(
        game_config,
        None,
        listener,
        status,
        data,
        ids,
        loader,
        world,
    ));
    (port, ServerGuard(Some(server.abort_handle())))
}

/// Waits until bot A is in play (its task finished early = a failure).
pub async fn wait_for_a(shared: &Shared, a: &tokio::task::JoinHandle<BotReport>) -> bool {
    loop {
        if shared.a_joined.load(Ordering::SeqCst) {
            return true;
        }
        if a.is_finished() {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Runs one scenario on our server: A joins first, B second; returns A's
/// trace path and one fact line per bot.
pub async fn run_scene(
    env: &Arc<ParityEnv>,
    ids: &Arc<PacketIds>,
    port: u16,
    scene: Scene,
) -> (PathBuf, Vec<String>) {
    let code = scene.code();
    let a_path = Path::new(TRACE_DIR).join(format!("two-{code}-ours-A.jsonl"));
    let b_path = Path::new(TRACE_DIR).join(format!("two-{code}-ours-B.jsonl"));
    let shared = Arc::new(Shared::new());
    let b_uuid = protocol::offline_uuid(BOT_B);

    let a = tokio::spawn(run_bot_at(
        Arc::clone(env),
        Arc::clone(ids),
        port,
        BOT_A,
        a_path.clone(),
        SAFETY,
        Observer::new(Arc::clone(&shared), b_uuid),
    ));
    let joined = tokio::time::timeout(JOIN_WAIT, wait_for_a(&shared, &a))
        .await
        .unwrap_or(false);
    let b = if joined {
        Some(tokio::spawn(run_bot_at(
            Arc::clone(env),
            Arc::clone(ids),
            port,
            BOT_B,
            b_path,
            SAFETY,
            BScript::new(scene, Arc::clone(&shared)),
        )))
    } else {
        None
    };
    let ra = a.await.expect("A join task");
    let mut facts = vec![format!("{code}: A {}", ra.end.text())];
    match b {
        Some(handle) => {
            let rb = handle.await.expect("B task");
            facts.push(format!("{code}: B {}", rb.end.text()));
        }
        None => facts.push(format!("{code}: B was never started (A did not join)")),
    }
    (a_path, facts)
}
