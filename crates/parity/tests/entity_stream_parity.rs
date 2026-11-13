//! M4.5 Шаг 4 (в): the entity stream about the second player (B), seen by
//! bot A on **our own** server, against the vanilla two-bot trace.
//!
//! One ignored test starts our in-process v3 server with real sockets (the
//! `window_walk_ours` pattern), runs the very same two-bot scenarios the
//! vanilla trace was recorded with (С1, С2, С4), and then compares A's
//! clientbound packets about B with the recorded vanilla ones:
//!
//! * B's entity id is identified from the `add_entity` whose UUID is B's
//!   offline UUID — never from a hard-coded number (the id differs per run,
//!   and in С4 the earlier analysis mistook a mob for B);
//! * the **type sequence** must be exactly equal;
//! * every payload must be **byte for byte** equal after normalising only the
//!   values the owner allows — the entity id, the position fields (a packet is
//!   emitted on a different tick) and the `player_info_update` game mode (our
//!   server is not in spectator);
//! * anything else is printed as an explicit list `type, offset, our byte,
//!   vanilla byte` — nothing is silently normalised.
//!
//! The run is expected to be **red** until the entity stream matches the
//! trace. The recorded divergences (С1/С2/С4, `docs/ARCH-M4.md` §6) are: our
//! stream misses vanilla's periodic zero-delta `move_entity_pos` while a
//! player stands and its `entity_position_sync` on rotation; our
//! `update_attributes` uses the С2 key order (8, 26, 13) where С1 records
//! (26, 13, 8); a world exit sends `player_info_remove` before
//! `remove_entities` where the trace has `remove_entities` first; and С2/С4
//! differ in the `rotate_head`/`move_entity_rot` cadence and rotation bytes.
//! The failure message points at the written report; the diffs are the (в)
//! finding, not a silent pass.
//!
//! ```text
//! cargo test -p parity --release --test entity_stream_parity -- --ignored --nocapture
//! ```
//!
//! Run it with `--release`: the v3 world read is CPU-bound (~13.5 s release vs
//! ~85 s debug). The traces land under `C:\MineRust-data\traces` next to the
//! vanilla `two-c{1,2,4}-A.jsonl`.

#[path = "common/twobot_scenarios.rs"]
#[allow(dead_code)]
mod scenarios;
#[path = "common/twobot.rs"]
#[allow(dead_code)]
mod twobot;

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use parity::{from_base64, load_parity_env, ParityEnv};
use protocol::{
    AddEntity, EntityPositionSync, MoveEntityPos, MoveEntityPosRot, MoveEntityRot, PacketReader,
    PlayerInfoRemove, PlayerInfoUpdate, RemoveEntities, RotateHead, SetEntityData,
    UpdateAttributes, Uuid,
};
use scenarios::{BScript, Observer, Scene};
use twobot::{run_bot_at, Shared, BOT_A, BOT_B};

/// Where the traces and the diff report land (outside the repository).
const TRACE_DIR: &str = r"C:\MineRust-data\traces";
/// The frozen v3 world, read-only for our in-process server.
const V3_TEMPLATE: &str = r"C:\MineRust-data\world-template-v3";
/// Budget for a bot to reach play before the scenario gives up.
const JOIN_WAIT: Duration = Duration::from_secs(180);
/// Per-bot play budget: the script plus a margin.
const SAFETY: Duration = Duration::from_secs(240);

/// One clientbound packet of a JSONL trace, decoded back to bytes.
#[derive(Clone)]
struct Line {
    /// Packet name from the packets report, when resolvable.
    name: Option<String>,
    /// `dir` of the line (`clientbound` / `serverbound`).
    dir: String,
    /// The payload bytes (base64-decoded).
    bytes: Vec<u8>,
}

/// Reads one JSONL trace. Panics on a malformed line: a broken trace is a
/// reading failure, not a scenario to guess around.
fn load_lines(path: &Path) -> Vec<Line> {
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
        let dir = value["dir"].as_str().unwrap_or_default().to_owned();
        let name = value
            .get("name")
            .and_then(|n| n.as_str())
            .map(str::to_owned);
        let seq = value["seq"].as_u64().unwrap_or(0);
        let bytes = from_base64(value["b64"].as_str().unwrap_or_default())
            .unwrap_or_else(|| panic!("bad base64 in line {seq}"));
        out.push(Line { name, dir, bytes });
    }
    out
}

/// The entity id B owns in a trace: the id of the `add_entity` whose UUID is
/// B's offline UUID. `None` when B never spawned for this viewer (a fact, not
/// a default).
fn b_entity_id(lines: &[Line], b_uuid: Uuid) -> Option<i32> {
    for line in lines {
        if line.name.as_deref() != Some("minecraft:add_entity") {
            continue;
        }
        if let Ok(entity) = AddEntity::decode_payload(&mut PacketReader::new(&line.bytes)) {
            if entity.uuid == b_uuid {
                return Some(entity.entity_id);
            }
        }
    }
    None
}

/// The leading VarInt of a payload (an entity id), when it decodes.
fn leading_id(bytes: &[u8]) -> Option<i32> {
    protocol::decode_varint(bytes).ok().map(|(value, _)| value)
}

/// Entity id a packet is about, per the observed layout (leading VarInt, the
/// `entity_event` event byte before it). Returns `None` for `remove_entities`
/// (a list) and the `player_info_*` packets (no id), handled by the caller.
fn entity_id_of(name: &str, bytes: &[u8]) -> Option<i32> {
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
fn remove_ids(bytes: &[u8]) -> Vec<i32> {
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
fn about_b(lines: &[Line], b_id: i32, b_uuid: Uuid) -> Vec<Line> {
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

/// Normalises the values the owner allows to differ: the entity id, the
/// position fields (a packet is emitted on a different tick) and the
/// `player_info_update` game mode. Everything else stays byte for byte.
fn normalized(name: &str, raw: &[u8], game_mode: &mut Option<i32>) -> Vec<u8> {
    match name {
        "minecraft:add_entity" => edit::<AddEntity, _>(raw, |entity| {
            entity.entity_id = 0;
            entity.x = 0.0;
            entity.y = 0.0;
            entity.z = 0.0;
        }),
        "minecraft:move_entity_pos" => edit::<MoveEntityPos, _>(raw, |packet| {
            packet.entity_id = 0;
            packet.dx = 0;
            packet.dy = 0;
            packet.dz = 0;
        }),
        "minecraft:move_entity_pos_rot" => edit::<MoveEntityPosRot, _>(raw, |packet| {
            packet.entity_id = 0;
            packet.dx = 0;
            packet.dy = 0;
            packet.dz = 0;
        }),
        "minecraft:move_entity_rot" => edit::<MoveEntityRot, _>(raw, |packet| packet.entity_id = 0),
        "minecraft:rotate_head" => edit::<RotateHead, _>(raw, |packet| packet.entity_id = 0),
        "minecraft:entity_position_sync" => edit::<EntityPositionSync, _>(raw, |packet| {
            packet.entity_id = 0;
            packet.x = 0.0;
            packet.y = 0.0;
            packet.z = 0.0;
        }),
        "minecraft:set_entity_data" => edit::<SetEntityData, _>(raw, |packet| packet.entity_id = 0),
        "minecraft:update_attributes" => {
            edit::<UpdateAttributes, _>(raw, |packet| packet.entity_id = 0)
        }
        "minecraft:remove_entities" => edit::<RemoveEntities, _>(raw, |packet| {
            packet.entity_ids.fill(0);
        }),
        "minecraft:player_info_update" => {
            let mut reader = PacketReader::new(raw);
            match PlayerInfoUpdate::decode_payload(&mut reader) {
                Ok(mut update) if reader.is_empty() => {
                    if let Some(entry) = update.players.first() {
                        *game_mode = Some(entry.game_mode);
                    }
                    for entry in &mut update.players {
                        entry.game_mode = 0;
                    }
                    update.encode_payload().unwrap_or_else(|_| raw.to_vec())
                }
                _ => raw.to_vec(),
            }
        }
        _ => raw.to_vec(),
    }
}

/// The entity packets expose inherent `encode_payload` / `decode_payload`; the
/// normaliser needs them behind a single bound, so this local shim names them.
trait Payload: Sized {
    /// Decodes a payload (packet id already consumed).
    fn decode(reader: &mut PacketReader<'_>) -> Result<Self, protocol::CodecError>;
    /// Re-encodes the payload (packet id excluded).
    fn encode(&self) -> Result<Vec<u8>, protocol::CodecError>;
}

macro_rules! payload_codec {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Payload for $ty {
                fn decode(reader: &mut PacketReader<'_>) -> Result<Self, protocol::CodecError> {
                    <$ty>::decode_payload(reader)
                }
                fn encode(&self) -> Result<Vec<u8>, protocol::CodecError> {
                    self.encode_payload()
                }
            }
        )*
    };
}

payload_codec!(
    AddEntity,
    MoveEntityPos,
    MoveEntityPosRot,
    MoveEntityRot,
    RotateHead,
    EntityPositionSync,
    SetEntityData,
    UpdateAttributes,
    RemoveEntities,
    PlayerInfoUpdate,
);

/// Decodes `raw` as `T`, applies `f`, and re-encodes — the identity when the
/// payload decodes cleanly. A payload that does not decode is returned as is,
/// so a layout mismatch still shows up as a byte difference.
fn edit<T, F>(raw: &[u8], f: F) -> Vec<u8>
where
    T: Payload,
    F: FnOnce(&mut T),
{
    let mut reader = PacketReader::new(raw);
    match T::decode(&mut reader) {
        Ok(mut value) if reader.is_empty() => {
            f(&mut value);
            value.encode().unwrap_or_else(|_| raw.to_vec())
        }
        _ => raw.to_vec(),
    }
}

/// The explicit byte diff of one aligned pair: the first differing offset
/// (in the normalised payload), our byte and the vanilla byte.
fn byte_diff(ours: &[u8], vanilla: &[u8]) -> String {
    let common = ours.len().min(vanilla.len());
    for offset in 0..common {
        if ours[offset] != vanilla[offset] {
            return format!(
                "first byte diff at offset {offset}: ours 0x{:02x}, vanilla 0x{:02x}",
                ours[offset], vanilla[offset]
            );
        }
    }
    if ours.len() != vanilla.len() {
        return format!(
            "length differs: ours {} bytes, vanilla {} bytes",
            ours.len(),
            vanilla.len()
        );
    }
    "payloads are byte-identical after normalisation".to_owned()
}

/// Compares A's "about B" packets of one scenario; returns the diff lines
/// (empty = byte-identical type sequence and payloads).
fn compare(scene: &str, vanilla: &[Line], ours: &[Line]) -> Vec<String> {
    let mut report = Vec::new();
    let n = vanilla.len().max(ours.len());
    if vanilla.len() != ours.len() {
        report.push(format!(
            "{scene}: packets about B: vanilla {}, ours {}",
            vanilla.len(),
            ours.len()
        ));
    }
    for index in 0..n {
        let v = vanilla.get(index).and_then(|line| line.name.clone());
        let o = ours.get(index).and_then(|line| line.name.clone());
        if v != o {
            report.push(format!(
                "{scene}: packet #{index} type: vanilla {}, ours {}",
                v.as_deref().unwrap_or("<none>"),
                o.as_deref().unwrap_or("<none>"),
            ));
        }
    }
    for index in 0..vanilla.len().min(ours.len()) {
        let (v, o) = (&vanilla[index], &ours[index]);
        if v.name != o.name {
            continue;
        }
        let name = v.name.clone().unwrap_or_default();
        let mut v_mode = None;
        let mut o_mode = None;
        let v_bytes = normalized(&name, &v.bytes, &mut v_mode);
        let o_bytes = normalized(&name, &o.bytes, &mut o_mode);
        if v_bytes != o_bytes {
            report.push(format!(
                "{scene}: packet #{index} {name}: {}",
                byte_diff(&o_bytes, &v_bytes)
            ));
        }
        if let Some((vanilla_mode, ours_mode)) = v_mode.zip(o_mode) {
            if vanilla_mode != ours_mode {
                report.push(format!(
                    "{scene}: packet #{index} {name} game mode: vanilla {vanilla_mode}, ours {ours_mode}"
                ));
            }
        }
    }
    report
}

/// Starts our in-process v3 server at an ephemeral port, returning the port
/// and an abort guard. [`start_with_world`](window_walk_ours) is a sibling
/// test binary, so the (small) builder is repeated here.
async fn start_server(env: &ParityEnv, ids: Arc<PacketIds>) -> (u16, ServerGuard) {
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

/// Aborts the in-process server on every exit path, including a panic.
struct ServerGuard(Option<tokio::task::AbortHandle>);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(abort) = self.0.take() {
            abort.abort();
        }
    }
}

/// Waits until bot A is in play (its task finished early = a failure).
async fn wait_for_a(shared: &Shared, a: &tokio::task::JoinHandle<twobot::BotReport>) -> bool {
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
/// trace path and a fact line per bot.
async fn run_scene(
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
    if let Some(handle) = b {
        let rb = handle.await.expect("B task");
        facts.push(format!("{code}: B {}", rb.end.text()));
    } else {
        facts.push(format!("{code}: B was never started (A did not join)"));
    }
    (a_path, facts)
}

/// Шаг 4 (в): the entity stream about B on our server vs the vanilla trace.
#[tokio::test]
#[ignore = "requires the owner's installation (C:\\MineRust-data)"]
async fn entity_stream_matches_vanilla() {
    let env = match load_parity_env() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            return;
        }
    };
    if !Path::new(V3_TEMPLATE).is_dir() {
        eprintln!("SKIP: v3 template not found at {V3_TEMPLATE}");
        return;
    }
    let ids = Arc::new(env.packet_ids().expect("packet ids"));
    let env = Arc::new(env);
    let (_port, _guard) = start_server(&env, Arc::clone(&ids)).await;
    let port = _port;

    let b_uuid = protocol::offline_uuid(BOT_B);
    let mut report = Vec::new();
    let mut facts = Vec::new();
    for scene in [Scene::Join, Scene::Walk, Scene::Rotate] {
        let (ours_path, scene_facts) = run_scene(&env, &ids, port, scene).await;
        facts.extend(scene_facts);
        let vanilla_path = Path::new(TRACE_DIR).join(format!("two-{}-A.jsonl", scene.code()));
        if !vanilla_path.is_file() {
            report.push(format!(
                "{}: missing vanilla trace {}",
                scene.code(),
                vanilla_path.display()
            ));
            continue;
        }
        let vanilla = load_lines(&vanilla_path);
        let ours = load_lines(&ours_path);
        let Some(v_id) = b_entity_id(&vanilla, b_uuid) else {
            report.push(format!(
                "{}: B never spawned in the vanilla trace",
                scene.code()
            ));
            continue;
        };
        let Some(o_id) = b_entity_id(&ours, b_uuid) else {
            report.push(format!("{}: B never spawned in our trace", scene.code()));
            continue;
        };
        facts.push(format!(
            "{}: B entity id vanilla {v_id}, ours {o_id}",
            scene.code()
        ));
        let v_about = about_b(&vanilla, v_id, b_uuid);
        let o_about = about_b(&ours, o_id, b_uuid);
        report.extend(compare(scene.code(), &v_about, &o_about));
    }

    let text = format!(
        "# M4.5 Шаг 4 (в) — entity stream about B: our server vs vanilla\n\n\
         ## Facts\n{}\n\n## Divergences\n{}\n",
        facts.iter().map(|f| format!("- {f}\n")).collect::<String>(),
        if report.is_empty() {
            "none\n".to_owned()
        } else {
            report.iter().map(|r| format!("- {r}\n")).collect()
        }
    );
    let path = Path::new(TRACE_DIR).join("entity-stream-parity.md");
    let _ = std::fs::write(&path, &text);
    for fact in &facts {
        eprintln!("fact: {fact}");
    }
    for line in &report {
        eprintln!("diff: {line}");
    }
    eprintln!("report: {}", path.display());
    assert!(
        report.is_empty(),
        "the entity stream about B diverges from vanilla ({} diffs); see {}",
        report.len(),
        path.display()
    );
}
