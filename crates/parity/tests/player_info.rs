//! Byte-level check of `minecraft:player_info_update` against the vanilla
//! recording of the join slice.
//!
//! `cargo test -p parity --test player_info -- --ignored`
//!
//! Needs `target/parity/join-{vanilla,ours}.json` (recorded by `join_slice`).
//! The vanilla joining entry (mask `0xFF`, one player) is decoded with our
//! codec and encoded back: the bytes must be exactly the recorded payload,
//! and both traces must decode to the same packet.

use std::path::{Path, PathBuf};

use parity::{from_hex, Trace, TraceDirection, TraceState};
use protocol::{PacketReader, PlayerInfoUpdate};

/// Where traces land: `<workspace>/target/parity/`.
fn trace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("parity")
}

/// Reads every recorded `minecraft:player_info_update` payload as raw bytes.
fn recorded_payloads(path: &Path) -> Vec<Vec<u8>> {
    let trace = match Trace::load(path) {
        Ok(trace) => trace,
        Err(error) => {
            eprintln!(
                "SKIP: {} not readable ({error}); run the join slice first",
                path.display()
            );
            return Vec::new();
        }
    };
    let mut payloads = Vec::new();
    for entry in &trace.packets {
        if entry.state != TraceState::Play
            || entry.direction != TraceDirection::Clientbound
            || entry.name.as_deref() != Some("minecraft:player_info_update")
        {
            continue;
        }
        match from_hex(&entry.payload) {
            Ok(bytes) => payloads.push(bytes),
            Err(error) => eprintln!("FAIL: player_info payload is not hex: {error}"),
        }
    }
    payloads
}

/// Decodes a recorded packet with our codec and returns it for a
/// decode/encode comparison.
fn decoded(payload: &[u8]) -> Option<PlayerInfoUpdate> {
    let mut reader = PacketReader::new(payload);
    match PlayerInfoUpdate::decode_payload(&mut reader) {
        Ok(packet) => {
            reader.ensure_empty().expect("no trailing bytes");
            Some(packet)
        }
        Err(error) => {
            eprintln!("FAIL: cannot decode the recorded packet: {error}");
            None
        }
    }
}

#[test]
#[ignore = "needs join-{vanilla,ours}.json recorded by the join slice"]
fn joining_entry_reencodes_to_the_vanilla_bytes() {
    let vanilla = recorded_payloads(&trace_dir().join("join-vanilla.json"));
    // The vanilla recording: an empty all-actions update, the joining
    // entry, and a later latency-only update of one player.
    let Some(entry) = vanilla
        .iter()
        .find(|payload| payload.len() > 2 && payload[0] == 0xFF)
    else {
        eprintln!("SKIP: no joining entry (mask 0xFF) in the vanilla recording");
        return;
    };

    let packet = decoded(entry).expect("the vanilla joining entry must decode");
    let again = packet.encode_payload().expect("encodable");
    assert_eq!(
        again, *entry,
        "encode must reproduce the recorded vanilla bytes exactly"
    );

    // Our recording must decode to the same packet.
    let ours = recorded_payloads(&trace_dir().join("join-ours.json"));
    let Some(ours_entry) = ours.first() else {
        eprintln!("SKIP: no player_info packet in our recording");
        return;
    };
    let ours_packet = decoded(ours_entry).expect("our joining entry must decode");
    assert_eq!(
        ours_packet, packet,
        "our join entry must carry every field of the vanilla one"
    );
    assert_eq!(
        ours_packet.encode_payload().expect("encodable"),
        *entry,
        "our join entry must be byte-identical to the vanilla one"
    );
}
