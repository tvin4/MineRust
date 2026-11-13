//! M4.5 Шаг 4 (г): the tracking distance on our server against the vanilla С3
//! trace, and the world-exit packet order.
//!
//! One ignored test starts our in-process v3 server with real sockets and runs
//! the very С3 scenario the vanilla trace was recorded with (bot A stands, bot
//! B walks +x until A loses it, then walks back), then compares where the
//! server dropped and re-added B with the vanilla trace.
//!
//! **The distance is measured from B's own clock** (B's clientbound drop/re-add
//! against B's own serverbound moves) — [`entity_stream::tracking`] explains
//! why: each bot's `t_ms` starts at its own connection, so the analyzer's
//! `entities-detail` cross-trace numbers (Шаг 1.4: remove 154.6, re-add 154.4,
//! max visible 160.2) carry the whole offset between the two *real* clients'
//! connections. That offset was ~1.4 s in the vanilla recording; the geometric
//! vanilla numbers are remove **160.2**, re-add **160.0** (both read from
//! `two-c3-B.jsonl`). Our in-process bots connect ~0.1 s apart, so their naive
//! numbers land near the geometric ones by accident.
//!
//! The test asserts our geometric remove matches the vanilla geometric remove
//! within one bot step, that our geometric re-add falls in the Шаг 1.4 interval
//! `[154.4, 160.2]`, that the maximum visible distance matches within one step,
//! and that the world exit is the C1 order: `remove_entities` first, then
//! `player_info_remove` (the (г) fix to `push_remove`). The naive numbers and
//! the measured clock offset are printed for the record.
//!
//! ```text
//! cargo test -p parity --release --test entity_view_distance -- --ignored --nocapture
//! ```

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

use entity_stream::{
    about_b, b_entity_id, load_lines, run_scene, start_server, tracking, Line, Tracking, TRACE_DIR,
    V3_TEMPLATE,
};
use scenarios::Scene;

/// One bot step per tick (blocks) — the owner's tolerance «в пределах одного
/// шага бота».
const STEP: f64 = 0.2;
/// The Шаг 1.4 tracking interval `[re-add, remove]` the owner asks the re-add
/// to fall in.
const INTERVAL: (f64, f64) = (154.4, 160.2);

/// The last two "about B" packet names of a trace (the world-exit tail).
fn exit_tail(about: &[Line]) -> Vec<String> {
    let mut names: Vec<String> = about
        .iter()
        .map(|line| line.name.clone().unwrap_or_default())
        .collect();
    names.drain(..names.len().saturating_sub(2));
    names
}

/// One line for the record: geometric vs naive, and the clock offset.
fn describe(what: &str, t: &Tracking) -> String {
    let val = |x: Option<f64>| x.map_or("n/a".to_owned(), |v| format!("{v:.1}"));
    format!(
        "{what}: geometric remove {}, re-add {}, max visible {}; \
         naive remove {}, re-add {}; clock offset {} ms",
        val(t.geom_remove),
        val(t.geom_readd),
        val(t.max_visible),
        val(t.naive_remove),
        val(t.naive_readd),
        val(t.offset_ms),
    )
}

/// Шаг 4 (г): С3 tracking distance on our server vs the vanilla trace, and the
/// world-exit order.
#[tokio::test]
#[ignore = "requires the owner's installation (C:\\MineRust-data)"]
async fn view_distance_matches_vanilla() {
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
    let (port, _guard) = start_server(&env, Arc::clone(&ids)).await;

    let a_uuid = protocol::offline_uuid(twobot::BOT_A);
    let b_uuid = protocol::offline_uuid(twobot::BOT_B);

    // Our run.
    let (ours_path, facts) = run_scene(&env, &ids, port, Scene::Track).await;
    for fact in &facts {
        eprintln!("fact: {fact}");
    }
    let ours_a = load_lines(&ours_path);
    let ours_b = load_lines(&Path::new(TRACE_DIR).join("two-c3-ours-B.jsonl"));
    let ours = tracking(&ours_a, &ours_b, a_uuid, b_uuid);
    eprintln!("{}", describe("С3 ours", &ours));

    // The vanilla recording of the same scenario.
    let vanilla_a = load_lines(&Path::new(TRACE_DIR).join("two-c3-A.jsonl"));
    let vanilla_b = load_lines(&Path::new(TRACE_DIR).join("two-c3-B.jsonl"));
    let vanilla = tracking(&vanilla_a, &vanilla_b, a_uuid, b_uuid);
    eprintln!("{}", describe("С3 vanilla", &vanilla));

    let (Some(ours_remove), Some(vanilla_remove)) = (ours.geom_remove, vanilla.geom_remove) else {
        panic!("both runs must drop B out of range while B walks away");
    };
    assert!(
        vanilla.geom_readd.is_some(),
        "the vanilla trace must re-add B when B walks back"
    );
    let Some(ours_readd) = ours.geom_readd else {
        panic!("our server must re-add B when B walks back");
    };
    // The world exit (the clean leave) must be remove_entities then
    // player_info_remove — the C1 order, compared against the C1 trace.
    let c1 = load_lines(&Path::new(TRACE_DIR).join("two-c1-A.jsonl"));
    let c1_id = b_entity_id(&c1, b_uuid).expect("B in the C1 trace");
    let c1_tail = exit_tail(&about_b(&c1, c1_id, b_uuid));
    let Some(ours_id) = b_entity_id(&ours_a, b_uuid) else {
        panic!("B must have spawned for A in our С3 trace");
    };
    let ours_tail = exit_tail(&about_b(&ours_a, ours_id, b_uuid));
    eprintln!("world-exit order: C1 {c1_tail:?}, ours {ours_tail:?}");
    assert_eq!(
        ours_tail, c1_tail,
        "the world exit must be remove_entities then player_info_remove (C1 order)"
    );

    assert!(
        (ours_remove - vanilla_remove).abs() <= STEP,
        "our server dropped B at {ours_remove:.1} blocks, vanilla at {vanilla_remove:.1} \
         (more than one bot step, {STEP})"
    );
    assert!(
        ours_readd >= INTERVAL.0 && ours_readd <= INTERVAL.1,
        "our server re-added B at {ours_readd:.1} blocks, outside the Шаг 1.4 interval \
         [{:.1}, {:.1}]",
        INTERVAL.0,
        INTERVAL.1
    );
    if let (Some(a), Some(b)) = (ours.max_visible, vanilla.max_visible) {
        assert!(
            (a - b).abs() <= STEP,
            "maximum visible distance ours {a:.1} vs vanilla {b:.1} exceeds one bot step ({STEP})"
        );
    }
}
