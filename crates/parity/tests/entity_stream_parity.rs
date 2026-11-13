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
//!   values the owner allows — the entity id and the position fields (a packet
//!   is emitted on a different tick). The game mode is **not** normalised since
//!   Шаг 7.3: the server runs each scenario with the mode the vanilla trace was
//!   recorded in (C1/C2/C4 = 3 spectator, C5 = 0 survival, C6 = 1 creative).
//! * anything else is printed as an explicit list `type, offset, our byte,
//!   vanilla byte` — nothing is silently normalised.
//!
//! The run is expected to be **red** until the entity stream matches the
//! trace. Шаг 6.3 (after the Шаг 6.2 cadence fix) leaves four residual
//! divergence classes, each **not derivable from the trace** (showing the data
//! and stopping on the item is the owner's rule; `docs/ARCH-M4.md` §6 «Шаг
//! 6.3» carries the numbers):
//!
//! * the `entity_position_sync` trigger (vanilla 8 in С2, 4 in С4; no period,
//!   step or threshold fits);
//! * the `rotate_head` byte rule — С2 shows head = the *next* update's yaw
//!   (56/65), С4 shows head = body − one client tick (45/50); contradictory,
//!   so no single rule follows;
//! * the `update_attributes` key order (С1 (26, 13, 8), С2 (8, 26, 13), С4
//!   (26, 8, 13) — unstable across vanilla runs, so not a rule);
//! * the zero-delta `move_entity_pos` period in С4 (gaps 38/53/116 ticks vs
//!   the strict 60 of С1/С2) and vanilla's 1/3/4-tick sampling jitter.
//!
//! Everything else matches: С1's type sequence and payloads are byte-identical
//! after the allowed normalisation except the attribute order. (The
//! world-exit order this test recorded — `player_info_remove` before
//! `remove_entities` — was fixed in Шаг 4 (г).)
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

#[path = "common/entity_stream.rs"]
#[allow(dead_code)]
mod entity_stream;
#[path = "common/twobot_scenarios.rs"]
#[allow(dead_code)]
mod scenarios;
#[path = "common/twobot.rs"]
#[allow(dead_code)]
mod twobot;

use std::path::Path;
use std::sync::Arc;

use parity::load_parity_env;
use protocol::{
    AddEntity, EntityPositionSync, MoveEntityPos, MoveEntityPosRot, MoveEntityRot, PacketReader,
    PlayerInfoUpdate, RemoveEntities, RotateHead, SetEntityData, UpdateAttributes,
};

use entity_stream::{
    about_b, b_entity_id, load_lines, run_scene, start_server, Line, TRACE_DIR, V3_TEMPLATE,
};
use scenarios::Scene;

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

/// Normalises the values the owner allows to differ: the entity id and the
/// position fields (a packet is emitted on a different tick). Everything else
/// stays byte for byte — since Шаг 7.3 the game mode is **not** normalised:
/// our server announces the player's own mode, so a mode byte that differs is
/// a real divergence.
fn normalized(name: &str, raw: &[u8]) -> Vec<u8> {
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
/// (empty = identical type sequence and byte-for-byte payloads).
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
        let v_bytes = normalized(&name, &v.bytes);
        let o_bytes = normalized(&name, &o.bytes);
        if v_bytes != o_bytes {
            report.push(format!(
                "{scene}: packet #{index} {name}: {}",
                byte_diff(&o_bytes, &v_bytes)
            ));
        }
    }
    report
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

    let b_uuid = protocol::offline_uuid(twobot::BOT_B);
    let mut report = Vec::new();
    let mut facts = Vec::new();
    // Шаг 7.3: the game mode is a server parameter and is no longer
    // normalised. C1/C2/C4 were recorded with both bots in spectator, so
    // our server runs them with mode 3; C5 (survival) and C6 (creative)
    // repeat the C1 script with modes 0 and 1.
    let passes: [(i32, &[(&str, Scene)]); 3] = [
        (
            3,
            &[
                ("c1", Scene::Join),
                ("c2", Scene::Walk),
                ("c4", Scene::Rotate),
            ],
        ),
        (0, &[("c5", Scene::Join)]),
        (1, &[("c6", Scene::Join)]),
    ];
    for (game_mode, scenes) in passes {
        let (port, _guard) = start_server(&env, Arc::clone(&ids), game_mode).await;
        for &(code, scene) in scenes {
            let (ours_path, scene_facts) = run_scene(&env, &ids, port, scene, code).await;
            facts.extend(scene_facts);
            let vanilla_path = Path::new(TRACE_DIR).join(format!("two-{code}-A.jsonl"));
            if !vanilla_path.is_file() {
                report.push(format!(
                    "{code}: missing vanilla trace {}",
                    vanilla_path.display()
                ));
                continue;
            }
            let vanilla = load_lines(&vanilla_path);
            let ours = load_lines(&ours_path);
            let Some(v_id) = b_entity_id(&vanilla, b_uuid) else {
                report.push(format!("{code}: B never spawned in the vanilla trace"));
                continue;
            };
            let Some(o_id) = b_entity_id(&ours, b_uuid) else {
                report.push(format!("{code}: B never spawned in our trace"));
                continue;
            };
            facts.push(format!(
                "{code}: game mode {game_mode}, B entity id vanilla {v_id}, ours {o_id}"
            ));
            let v_about = about_b(&vanilla, v_id, b_uuid);
            let o_about = about_b(&ours, o_id, b_uuid);
            report.extend(compare(code, &v_about, &o_about));
        }
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
