//! Inspection of the saved vanilla trace: the registries and tags a real
//! 26.3 server sends, next to what our server sends today.
//!
//! `cargo test -p parity --test trace_inspect -- --ignored --nocapture`
//!
//! Needs `target/parity/join-vanilla.json` (recorded by `join_slice`) and a
//! `client_jar` in `server.toml`. Nothing is asserted here: this is the
//! measurement the registry/tag fixes are based on (the "before" table).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parity::{from_hex, load_parity_env, ParityEnv, Trace, TraceDirection, TraceState};
use protocol::PacketReader;

/// Where traces land: `<workspace>/target/parity/`.
fn trace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("parity")
}

/// Reads the protocol String at the front (VarInt length + UTF-8) and
/// returns it with the rest of the payload.
fn take_string(payload: &[u8]) -> Option<(String, &[u8])> {
    let (len, used) = protocol::decode_varint(payload).ok()?;
    let len = usize::try_from(len).ok()?;
    let end = used.checked_add(len)?;
    let head = payload.get(used..end)?;
    let tail = payload.get(end..)?;
    let text = std::str::from_utf8(head).ok()?.to_owned();
    Some((text, tail))
}

/// Collects registry_data registry ids (the leading string of each packet).
fn vanilla_registry_ids(trace: &Trace) -> Vec<String> {
    trace
        .packets
        .iter()
        .filter(|entry| {
            entry.state == TraceState::Configuration
                && entry.direction == TraceDirection::Clientbound
                && entry.id == protocol::RegistryData::ID
        })
        .filter_map(|entry| take_string(&from_hex(&entry.payload).ok()?).map(|(name, _)| name))
        .collect()
}

/// Decodes the vanilla update_tags payload; decoding failure is reported
/// (it would itself be a parity finding).
fn vanilla_tags(trace: &Trace) -> Option<protocol::UpdateTags> {
    let entry = trace.packets.iter().find(|entry| {
        entry.state == TraceState::Configuration
            && entry.direction == TraceDirection::Clientbound
            && entry.id == protocol::UpdateTags::ID
    })?;
    let payload = match from_hex(&entry.payload) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("update_tags payload is not hex: {error}");
            return None;
        }
    };
    let mut reader = PacketReader::new(&payload);
    match protocol::UpdateTags::decode_payload(&mut reader) {
        Ok(tags) => Some(tags),
        Err(error) => {
            eprintln!("cannot decode vanilla update_tags: {error}");
            None
        }
    }
}

/// Summarises a decoded tag packet: registry -> (tag count, entry count).
fn tag_counts(registries: &[protocol::TagRegistry]) -> BTreeMap<&str, (usize, usize)> {
    registries
        .iter()
        .map(|registry| {
            let entries: usize = registry.tags.iter().map(|tag| tag.entries.len()).sum();
            (
                registry.registry_id.as_str(),
                (registry.tags.len(), entries),
            )
        })
        .collect()
}

/// Prints the markdown tables: registry_data diff and update_tags diff.
fn print_tables(env: &ParityEnv, vanilla: &Trace) {
    println!("## registry_data: состав реестров\n");
    let mut vanilla_ids = vanilla_registry_ids(vanilla);
    vanilla_ids.sort();
    let ours: Vec<String> = data::SYNCHRONIZED_REGISTRIES
        .iter()
        .map(|dir| format!("minecraft:{dir}"))
        .collect();
    let missing: Vec<&String> = vanilla_ids
        .iter()
        .filter(|id| !ours.iter().any(|value| value == id.as_str()))
        .collect();
    println!("у ванили: {}, у нас: {}", vanilla_ids.len(), ours.len());
    println!("отсутствуют у нас ({}):", missing.len());
    for id in missing {
        println!("  - {id}");
    }
    println!();

    println!("## update_tags: теги по реестрам\n");
    println!("| Реестр | Тегов у ванили | Элементов у ванили | Тегов у нас | Элементов у нас |");
    println!("|---|---|---|---|---|");

    let vanilla_tags = vanilla_tags(vanilla);
    let vanilla_counts = vanilla_tags
        .as_ref()
        .map(|tags| tag_counts(&tags.registries))
        .unwrap_or_default();

    let game_data = data::load_from_jar(
        &env.config.client_jar,
        env.config.registries_report.as_deref().map(Path::new),
    )
    .expect("game data from the jar");
    let ours_counts: BTreeMap<&str, (usize, usize)> = game_data
        .tags()
        .iter()
        .map(|(registry_id, tags)| {
            let entries: usize = tags.values().map(Vec::len).sum();
            (registry_id.as_str(), (tags.len(), entries))
        })
        .collect();

    let mut names: Vec<&str> = vanilla_counts
        .keys()
        .chain(ours_counts.keys())
        .copied()
        .collect();
    names.sort_unstable();
    names.dedup();
    for name in names {
        let (vt, ve) = vanilla_counts.get(name).copied().unwrap_or((0, 0));
        let (ot, oe) = ours_counts.get(name).copied().unwrap_or((0, 0));
        println!("| `{name}` | {vt} | {ve} | {ot} | {oe} |");
    }
}

#[test]
#[ignore = "needs join-vanilla.json and the owner's client jar"]
fn prints_registry_and_tag_tables() {
    let env = match load_parity_env() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            return;
        }
    };
    let path = trace_dir().join("join-vanilla.json");
    let vanilla = match Trace::load(&path) {
        Ok(trace) => trace,
        Err(error) => {
            eprintln!(
                "SKIP: {} not readable ({error}); run the join slice first",
                path.display()
            );
            return;
        }
    };
    print_tables(&env, &vanilla);
}
