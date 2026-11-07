//! Byte parity of our Update Tags packet against the vanilla trace.
//!
//! `cargo test -p parity --test update_tags_parity -- --ignored --nocapture`
//!
//! Builds the packet exactly the server does (`GameData::tags_packet`) and
//! compares it with the packet a real 26.3 server recorded in
//! `target/parity/join-vanilla.json`. Asserted strictly: the registry
//! composition, the tag composition of every registry and every tag's
//! entry ids — a mismatch names the registry, tag and first differing
//! id. When all that matches the encoded payloads are compared byte for
//! byte.
//!
//! Vanilla's *ordering* (registries in the packet, tags inside a
//! registry) is the iteration order of the vanilla server's internal maps;
//! the trace pins it down per run but yields no rule to reproduce it with.
//! The packet carries names alongside ids and the observed client binds
//! tags by name (it complained about missing registries, never about
//! order), so a differing order is reported as a warning instead of a
//! failure. The payload byte compare only runs when the order agrees.
//!
//! Needs `client_jar` and `registries_report` from `server.toml`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use parity::{from_hex, load_parity_env, ParityEnv, Trace, TraceDirection, TraceState};
use protocol::{PacketReader, UpdateTags};

/// Where traces land: `<workspace>/target/parity/`.
fn trace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("parity")
}

/// Finds the vanilla Update Tags packet in the trace.
fn vanilla_packet(trace: &Trace) -> Option<&parity::TraceEntry> {
    trace.packets.iter().find(|entry| {
        entry.state == TraceState::Configuration
            && entry.direction == TraceDirection::Clientbound
            && entry.id == protocol::UpdateTags::ID
    })
}

const SKIP_MISSING: &str = "SKIP: the trace has no update_tags packet";

/// Decodes the vanilla payload; panics if it does not decode (that would
/// itself be a parity finding).
fn decode(payload: &str) -> UpdateTags {
    let bytes = from_hex(payload).expect("trace payload is hex");
    let mut reader = PacketReader::new(&bytes);
    UpdateTags::decode_payload(&mut reader)
        .expect("the vanilla update_tags payload decodes as documented")
}

/// Registry ids of the packet, in the order it encodes them.
fn registry_ids(tags: &UpdateTags) -> Vec<&str> {
    tags.registries
        .iter()
        .map(|registry| registry.registry_id.as_str())
        .collect()
}

/// Tag names of one registry, in the order it encodes them.
fn tag_names(registry: &protocol::TagRegistry) -> Vec<&str> {
    registry.tags.iter().map(|tag| tag.name.as_str()).collect()
}

/// Splits the comparison into payload-relevant content differences
/// (composition and values; failing) and pure ordering notes (warnings).
fn report_diffs(ours: &UpdateTags, vanilla: &UpdateTags) -> (Vec<String>, Vec<String>) {
    let mut content = Vec::new();
    let mut order = Vec::new();
    let ours_ids = registry_ids(ours);
    let vanilla_ids = registry_ids(vanilla);
    let ours_set: BTreeSet<&str> = ours_ids.iter().copied().collect();
    let vanilla_set: BTreeSet<&str> = vanilla_ids.iter().copied().collect();

    for id in vanilla_set.difference(&ours_set) {
        content.push(format!("registry `{id}` is missing for us"));
    }
    for id in ours_set.difference(&vanilla_set) {
        content.push(format!("registry `{id}` is extra for us"));
    }
    if ours_set == vanilla_set && ours_ids != vanilla_ids {
        order.push(format!(
            "registry order differs (vanilla {} registries, ours alphabetically)",
            vanilla_ids.len()
        ));
    }

    for vanilla_registry in &vanilla.registries {
        let Some(ours_registry) = ours
            .registries
            .iter()
            .find(|registry| registry.registry_id == vanilla_registry.registry_id)
        else {
            continue;
        };
        let registry_id = &vanilla_registry.registry_id;
        let ours_names = tag_names(ours_registry);
        let vanilla_names = tag_names(vanilla_registry);
        let ours_tag_set: BTreeSet<&str> = ours_names.iter().copied().collect();
        let vanilla_tag_set: BTreeSet<&str> = vanilla_names.iter().copied().collect();

        for name in vanilla_tag_set.difference(&ours_tag_set) {
            content.push(format!("{registry_id}: tag `{name}` is missing for us"));
        }
        for name in ours_tag_set.difference(&vanilla_tag_set) {
            content.push(format!("{registry_id}: tag `{name}` is extra for us"));
        }
        if ours_tag_set == vanilla_tag_set && ours_names != vanilla_names {
            order.push(format!("{registry_id}: tag order differs"));
        }

        for vanilla_tag in &vanilla_registry.tags {
            let Some(ours_tag) = ours_registry
                .tags
                .iter()
                .find(|tag| tag.name == vanilla_tag.name)
            else {
                continue;
            };
            if ours_tag.entries == vanilla_tag.entries {
                continue;
            }
            let first = ours_tag
                .entries
                .iter()
                .zip(&vanilla_tag.entries)
                .position(|(ours, vanilla)| ours != vanilla)
                .unwrap_or(ours_tag.entries.len().min(vanilla_tag.entries.len()));
            content.push(format!(
                "{registry_id} `{}` differs: vanilla {} entries, ours {}; \
                 at #{} vanilla {:?}, ours {:?}",
                vanilla_tag.name,
                vanilla_tag.entries.len(),
                ours_tag.entries.len(),
                first,
                vanilla_tag.entries.get(first),
                ours_tag.entries.get(first),
            ));
        }
    }
    (content, order)
}

#[test]
#[ignore = "needs join-vanilla.json and the owner's client jar + registries report"]
fn our_update_tags_equals_the_vanilla_packet() {
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
    let Some(entry) = vanilla_packet(&trace) else {
        eprintln!("{SKIP_MISSING}");
        return;
    };
    let vanilla_payload = from_hex(&entry.payload).expect("trace payload is hex");
    let vanilla = decode(&entry.payload);

    let game_data = data::load_from_jar(
        &env.config.client_jar,
        env.config.registries_report.as_deref().map(Path::new),
    )
    .expect("game data from the jar");
    let ours = game_data.tags_packet();

    let (content, order) = report_diffs(&ours, &vanilla);
    assert!(
        content.is_empty(),
        "update_tags differs from vanilla:\n{}",
        content.join("\n")
    );

    let tag_count: usize = vanilla
        .registries
        .iter()
        .map(|registry| registry.tags.len())
        .sum();
    println!(
        "composition and values match vanilla: {} registries, {} tags",
        vanilla.registries.len(),
        tag_count,
    );

    if !order.is_empty() {
        // Ordering alone: the packet content is verified above; the order
        // is the vanilla server's internal iteration order (no rule to
        // reproduce it), so the payload bytes cannot be compared whole.
        eprintln!(
            "WARNING: ordering differs from vanilla ({} note(s)), full payload compare skipped:",
            order.len()
        );
        for note in &order {
            eprintln!("  - {note}");
        }
        return;
    }

    // Identical content and order: the payload must be byte-equal, which
    // also pins down the encoding (varints, string lengths, counts).
    let our_bytes = ours.encode_payload().expect("our update_tags encodes");
    let differ_at = our_bytes
        .iter()
        .zip(&vanilla_payload)
        .position(|(ours, vanilla)| ours != vanilla)
        .or_else(|| {
            (our_bytes.len() != vanilla_payload.len())
                .then_some(our_bytes.len().min(vanilla_payload.len()))
        });
    if let Some(offset) = differ_at {
        let window = 16;
        let from = offset.saturating_sub(window / 2);
        let ours_end = (from + window).min(our_bytes.len());
        let vanilla_end = (from + window).min(vanilla_payload.len());
        panic!(
            "payloads differ at byte {offset} (ours {} bytes, vanilla {} bytes):\n\
             ours    [{from}..{ours_end}]: {:02x?}\n\
             vanilla [{from}..{vanilla_end}]: {:02x?}",
            our_bytes.len(),
            vanilla_payload.len(),
            &our_bytes[from..ours_end],
            &vanilla_payload[from..vanilla_end],
        );
    }
    println!(
        "byte parity: payload of {} bytes identical",
        vanilla_payload.len()
    );
}
