//! Registry Data parity: the 32 registries a real 26.3 server sends in the
//! configuration state against what our server loads from the client jar.
//!
//! `cargo test -p parity --test registry_data_parity -- --ignored --nocapture`
//!
//! Compares the *content*: the set of registry identifiers, and inside every
//! registry the set of entry keys, their count and their order (the packet
//! turns an entry's position into its numeric id, so the order is part of
//! the contract). No byte comparison of the NBT payloads here — only names
//! and counts, as agreed with the owner.
//!
//! Needs `target/parity/join-vanilla.json` (recorded by `join_slice`) and a
//! `client_jar` in `server.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parity::{from_hex, load_parity_env, ParityEnv, Trace, TraceDirection, TraceState};
use protocol::{PacketReader, RegistryData};

/// Where traces land: `<workspace>/target/parity/`.
fn trace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("parity")
}

/// Decodes every vanilla Registry Data packet, in the order the server sent
/// them. A payload that does not decode panics: that is itself a finding.
fn vanilla_registries(trace: &Trace) -> Vec<RegistryData> {
    trace
        .packets
        .iter()
        .filter(|entry| {
            entry.state == TraceState::Configuration
                && entry.direction == TraceDirection::Clientbound
                && entry.id == protocol::RegistryData::ID
        })
        .map(|entry| {
            let bytes = from_hex(&entry.payload).expect("trace payload is hex");
            let mut reader = PacketReader::new(&bytes);
            RegistryData::decode_payload(&mut reader).expect("vanilla registry_data decodes")
        })
        .collect()
}

/// Prints the markdown table and returns the content findings.
fn compare(vanilla: &[RegistryData], ours: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let mut findings = Vec::new();
    let vanilla_ids: Vec<&str> = vanilla
        .iter()
        .map(|registry| registry.registry_id.as_str())
        .collect();
    let vanilla_set: std::collections::BTreeSet<&str> = vanilla_ids.iter().copied().collect();
    let ours_set: std::collections::BTreeSet<&str> = ours.keys().map(String::as_str).collect();

    for id in vanilla_set.difference(&ours_set) {
        findings.push(format!("registry `{id}` is missing for us"));
    }
    for id in ours_set.difference(&vanilla_set) {
        findings.push(format!("registry `{id}` is extra for us"));
    }
    if vanilla_set == ours_set && vanilla_ids.len() != vanilla_set.len() {
        findings.push("vanilla sends a registry twice".to_owned());
    }

    println!("| # | Реестр | Записей у ванили | Записей у нас | Ключи (множество) | Порядок |");
    println!("|---|---|---|---|---|---|");
    for (index, registry) in vanilla.iter().enumerate() {
        let id = registry.registry_id.as_str();
        let Some(ours_keys) = ours.get(id) else {
            println!(
                "| {} | `{}` | {} | — | отсутствует у нас | — |",
                index + 1,
                id,
                registry.entries.len()
            );
            continue;
        };
        let vanilla_keys: Vec<&str> = registry
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        let ours_str: Vec<&str> = ours_keys.iter().map(String::as_str).collect();
        let keys_equal = vanilla_keys == ours_str;
        let sets = {
            let vanilla_names: std::collections::BTreeSet<&str> =
                vanilla_keys.iter().copied().collect();
            let ours_names: std::collections::BTreeSet<&str> = ours_str.iter().copied().collect();
            vanilla_names == ours_names
        };
        if !sets {
            for name in sets_diff(&vanilla_keys, &ours_str) {
                findings.push(format!("{id}: `{name}`"));
            }
        }
        if sets && !keys_equal {
            findings.push(format!("{id}: entry order differs"));
        }
        println!(
            "| {} | `{}` | {} | {} | {} | {} |",
            index + 1,
            id,
            registry.entries.len(),
            ours_keys.len(),
            if sets {
                "да"
            } else {
                "РАСХОЖДЕНИЕ"
            },
            if keys_equal {
                "да"
            } else if sets {
                "РАСХОЖДЕНИЕ"
            } else {
                "—"
            },
        );
    }
    findings
}

/// Symmetric difference of two key lists, with a marker for which side.
fn sets_diff(left: &[&str], right: &[&str]) -> Vec<String> {
    let left_set: std::collections::BTreeSet<&str> = left.iter().copied().collect();
    let right_set: std::collections::BTreeSet<&str> = right.iter().copied().collect();
    left_set
        .difference(&right_set)
        .map(|name| format!("missing for us: {name}"))
        .chain(
            right_set
                .difference(&left_set)
                .map(|name| format!("extra for us: {name}")),
        )
        .collect()
}

#[test]
#[ignore = "needs join-vanilla.json and the owner's client jar"]
fn our_registry_data_matches_the_vanilla_composition() {
    let env: ParityEnv = match load_parity_env() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            return;
        }
    };
    let path = trace_dir().join("join-vanilla.json");
    let trace = match Trace::load(&path) {
        Ok(trace) => trace,
        Err(error) => {
            eprintln!(
                "SKIP: {} not readable ({error}); run the join slice first",
                path.display()
            );
            return;
        }
    };
    let vanilla = vanilla_registries(&trace);
    if vanilla.is_empty() {
        eprintln!("SKIP: the trace has no registry_data packets");
        return;
    }

    let game_data = data::load_from_jar(
        &env.config.client_jar,
        env.config.registries_report.as_deref().map(Path::new),
    )
    .expect("game data from the jar");
    let ours: BTreeMap<String, Vec<String>> = game_data
        .registries()
        .map(|(id, registry)| {
            let keys = registry
                .entries()
                .iter()
                .map(|entry| entry.name.clone())
                .collect();
            (id.to_owned(), keys)
        })
        .collect();

    let findings = compare(&vanilla, &ours);
    assert!(
        findings.is_empty(),
        "registry_data differs from vanilla ({} finding(s)):\n{}",
        findings.len(),
        findings.join("\n")
    );
    println!(
        "composition matches vanilla: {} registries, {} entries",
        vanilla.len(),
        vanilla
            .iter()
            .map(|registry| registry.entries.len())
            .sum::<usize>(),
    );
}
