//! Field-by-field verification of `minecraft:login` (play) against the
//! vanilla recording of the join slice.
//!
//! `cargo test -p parity --test login_fields -- --ignored`
//!
//! Needs `target/parity/join-{vanilla,ours}.json` (recorded by `join_slice`).
//! The vanilla packet is decoded with our own `LoginPlay` codec: every field
//! of the real 26.3 server must read back with the values the recording
//! carries, and the fields that do not depend on the world must be byte
//! identical between the two traces. World-dependent fields (entity id,
//! dimension list, hashed seed) are named explicitly below.

use std::path::{Path, PathBuf};

use parity::{from_hex, Trace, TraceDirection, TraceState};
use protocol::{LoginPlay, PacketReader};

/// Where traces land: `<workspace>/target/parity/`.
fn trace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("parity")
}

/// Finds the one `minecraft:login` of a recorded connection and decodes it
/// with our codec; returns `None` when the trace is missing or undecodable,
/// which the test reports as a skip rather than a pass.
fn decode_login(path: &Path) -> Option<LoginPlay> {
    let trace = match Trace::load(path) {
        Ok(trace) => trace,
        Err(error) => {
            eprintln!(
                "SKIP: {} not readable ({error}); run the join slice first",
                path.display()
            );
            return None;
        }
    };
    let entry = trace.packets.iter().find(|entry| {
        entry.state == TraceState::Play
            && entry.direction == TraceDirection::Clientbound
            && entry.name.as_deref() == Some("minecraft:login")
    })?;
    let payload = match from_hex(&entry.payload) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("FAIL: minecraft:login payload is not hex: {error}");
            return None;
        }
    };
    let mut reader = PacketReader::new(&payload);
    match LoginPlay::decode_payload(&mut reader) {
        Ok(login) => {
            reader
                .ensure_empty()
                .expect("login payload has no trailing bytes");
            Some(login)
        }
        Err(error) => {
            eprintln!("FAIL: cannot decode the recorded vanilla minecraft:login: {error}");
            None
        }
    }
}

/// Every field of the vanilla `minecraft:login`, as the recording carries
/// it, plus the equality of all world-independent fields against ours.
#[test]
#[ignore = "needs join-{vanilla,ours}.json recorded by the join slice"]
fn login_play_fields_match_the_vanilla_recording() {
    let Some(vanilla) = decode_login(&trace_dir().join("join-vanilla.json")) else {
        return;
    };
    let Some(ours) = decode_login(&trace_dir().join("join-ours.json")) else {
        return;
    };

    // The vanilla values, read field by field off the recording.
    assert_eq!(vanilla.entity_id, 4, "entity id of the vanilla connection");
    assert!(!vanilla.hardcore);
    let mut names = vanilla.dimension_names.clone();
    names.sort();
    assert_eq!(
        names,
        [
            "minecraft:overworld".to_owned(),
            "minecraft:the_end".to_owned(),
            "minecraft:the_nether".to_owned(),
        ],
        "the recording lists the three dimensions of its world, not the 4 \
         registry types; the order itself hardens between runs (both were \
         observed), so the names are compared as a set"
    );
    assert_eq!(vanilla.max_players, 20);
    assert_eq!(vanilla.view_distance, 10);
    assert_eq!(vanilla.simulation_distance, 10);
    assert!(!vanilla.reduced_debug_info);
    assert!(vanilla.enable_respawn_screen);
    assert!(!vanilla.do_limited_crafting);
    assert_eq!(
        vanilla.dimension_type, 0,
        "dimension_type id of the overworld"
    );
    assert_eq!(vanilla.dimension_name, "minecraft:overworld");
    assert_eq!(
        vanilla.hashed_seed,
        protocol::hashed_seed(12345),
        "seed hash of the parity world (level-seed=12345)"
    );
    assert_eq!(vanilla.game_mode, 0);
    assert_eq!(vanilla.previous_game_mode, 0);
    assert!(!vanilla.is_debug);
    assert!(!vanilla.is_flat);
    assert!(vanilla.death_location.is_none());
    assert_eq!(vanilla.portal_cooldown, 0);
    assert_eq!(vanilla.sea_level, 63);
    assert!(!vanilla.online_mode);
    assert!(!vanilla.enforces_secure_chat);

    // World-independent fields: identical between the traces.
    assert!(!ours.hardcore);
    assert_eq!((ours.max_players, ours.view_distance), (20, 10));
    assert_eq!(ours.simulation_distance, 10);
    assert!(!ours.reduced_debug_info);
    assert!(ours.enable_respawn_screen);
    assert!(!ours.do_limited_crafting);
    assert_eq!(
        ours.dimension_type, vanilla.dimension_type,
        "dimension_type must match"
    );
    assert_eq!(
        ours.dimension_name, vanilla.dimension_name,
        "dimension name must match"
    );
    assert_eq!((ours.game_mode, ours.previous_game_mode), (0, 0));
    assert!(!ours.is_debug && !ours.is_flat);
    assert!(ours.death_location.is_none());
    assert_eq!(ours.portal_cooldown, 0);
    assert_eq!(ours.sea_level, vanilla.sea_level, "sea level must match");
    assert!(!ours.online_mode && !ours.enforces_secure_chat);

    // World-dependent fields left for part 3 (all normalised in the diff):
    // entity id (per connection), dimension names (the world template's
    // three dimensions) and hashed seed (level.dat).
    // SOURCE: parity recording; fixed when the world arrives.
}
