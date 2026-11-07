//! Diff of two raw traces: the vanilla recording vs our recording.
//!
//! The comparison is byte level. Packets are grouped by
//! `(state, direction, id)`; the groups are compared by count and, inside a
//! group, the n-th vanilla packet against the n-th packet of ours. Field
//! parsing exists for exactly one purpose — normalising randomness before
//! the byte comparison (see [`normalize`]): a different port or a different
//! teleport id is not a server discrepancy.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::trace::{from_hex, to_hex, Trace, TraceDirection, TraceEntry, TraceState};

/// Identity of one packet kind inside a trace.
type PacketKey = (TraceState, TraceDirection, u32);

/// One packet kind whose occurrence counts differ between the traces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountRow {
    /// Protocol state of the packet.
    pub state: TraceState,
    /// Direction relative to the server.
    pub direction: TraceDirection,
    /// Numeric packet id on the wire.
    pub id: u32,
    /// Packet name from the packets report, when known.
    pub name: Option<String>,
    /// Occurrences in the vanilla trace.
    pub vanilla: usize,
    /// Occurrences in our trace.
    pub ours: usize,
}

/// Byte mismatch of the n-th occurrence of one packet kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PayloadRow {
    /// Protocol state of the packet.
    pub state: TraceState,
    /// Direction relative to the server.
    pub direction: TraceDirection,
    /// Numeric packet id on the wire.
    pub id: u32,
    /// Packet name from the packets report, when known.
    pub name: Option<String>,
    /// Which occurrence of the kind this pair is (1-based).
    pub occurrence: usize,
    /// Vanilla payload after normalisation.
    pub vanilla: Vec<u8>,
    /// Our payload after normalisation.
    pub ours: Vec<u8>,
}

/// The full comparison of two traces of one script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TracesDiff {
    /// Which side produced the first trace (usually `vanilla`).
    pub vanilla_target: String,
    /// Which side produced the second trace (usually `ours`).
    pub ours_target: String,
    /// Packets recorded by the first trace.
    pub vanilla_total: usize,
    /// Packets recorded by the second trace.
    pub ours_total: usize,
    /// Groups with an unequal number of occurrences (empty = same shape).
    pub counts: Vec<CountRow>,
    /// Pairs whose normalised bytes differ (empty = identical payloads).
    pub payloads: Vec<PayloadRow>,
    /// How many packet pairs were byte-compared in total.
    pub compared: usize,
}

impl TracesDiff {
    /// Whether both traces are byte-identical after normalisation.
    pub fn is_identical(&self) -> bool {
        self.counts.is_empty() && self.payloads.is_empty()
    }
}

/// Compares `vanilla` with `ours` packet by packet.
pub fn diff_traces(vanilla: &Trace, ours: &Trace) -> TracesDiff {
    let vanilla_groups = group(&vanilla.packets);
    let ours_groups = group(&ours.packets);
    let keys: BTreeSet<PacketKey> = vanilla_groups
        .keys()
        .chain(ours_groups.keys())
        .copied()
        .collect();

    let mut counts = Vec::new();
    let mut payloads = Vec::new();
    let mut compared = 0;
    for key in keys {
        let left = vanilla_groups.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        let right = ours_groups.get(&key).map(Vec::as_slice).unwrap_or(&[]);
        let (state, direction, id) = key;
        let name = left
            .iter()
            .chain(right.iter())
            .find_map(|entry| entry.name.clone());
        if left.len() != right.len() {
            counts.push(CountRow {
                state,
                direction,
                id,
                name: name.clone(),
                vanilla: left.len(),
                ours: right.len(),
            });
        }
        for (index, (v_entry, o_entry)) in left.iter().zip(right).enumerate() {
            compared += 1;
            let v_norm = normalize(v_entry);
            let o_norm = normalize(o_entry);
            if v_norm != o_norm {
                payloads.push(PayloadRow {
                    state,
                    direction,
                    id,
                    name: name.clone(),
                    occurrence: index + 1,
                    vanilla: v_norm,
                    ours: o_norm,
                });
            }
        }
    }

    TracesDiff {
        vanilla_target: vanilla.target.clone(),
        ours_target: ours.target.clone(),
        vanilla_total: vanilla.len(),
        ours_total: ours.len(),
        counts,
        payloads,
        compared,
    }
}

/// Groups the packets in record order under `(state, direction, id)`.
fn group(entries: &[TraceEntry]) -> BTreeMap<PacketKey, Vec<&TraceEntry>> {
    let mut groups: BTreeMap<PacketKey, Vec<&TraceEntry>> = BTreeMap::new();
    for entry in entries {
        groups
            .entry((entry.state, entry.direction, entry.id))
            .or_default()
            .push(entry);
    }
    groups
}

/// Single value every per-connection counter is normalised to.
///
/// Entity ids and teleport ids are allocated per connection on both sides
/// and would therefore always differ between the recordings: within one
/// recorded connection they are replaced by this one value everywhere, so
/// only real content differences reach the comparison.
const NORMALIZED_ID: i32 = 0;

/// Spoils the random parts of one recorded payload so that only real
/// differences remain in the byte comparison.
///
/// Rules (kept as few as possible, each justified by the test setup):
///
/// * `minecraft:intention` — the port: vanilla is always reached on the
///   `server-port` of `server.properties`, our in-process server binds an
///   ephemeral port, so the port carries no parity information.
/// * Per-connection counters — teleport id (`minecraft:player_position`
///   clientbound, `minecraft:accept_teleportation` serverbound) and entity
///   id (`minecraft:add_entity`, `minecraft:teleport_entity`): each is the
///   payload's first VarInt and is replaced by [`NORMALIZED_ID`] throughout
///   the recording, one value for the whole connection.
/// * `minecraft:login_finished` — the session UUID (last 16 bytes): the
///   vanilla server draws a fresh random version-4 UUID per connection, so
///   the value carries no parity information; the tail is zeroed.
///
/// Everything else (names, coordinates, profile UUIDs) is deliberately
/// **not** normalised: differences there are what the owner has to see.
pub fn normalize(entry: &TraceEntry) -> Vec<u8> {
    let raw = match from_hex(&entry.payload) {
        Ok(bytes) => bytes,
        // A broken hex string is the recording side's failure; keep the
        // bytes as they are so the pair still shows up as a difference.
        Err(_) => entry.payload.as_bytes().to_vec(),
    };
    match entry.name.as_deref() {
        Some("minecraft:intention") => normalize_port(&raw).unwrap_or(raw),
        Some("minecraft:login_finished") => normalize_session_uuid(&raw),
        Some("minecraft:player_position")
        | Some("minecraft:accept_teleportation")
        | Some("minecraft:add_entity")
        | Some("minecraft:teleport_entity") => normalize_first_varint(&raw),
        _ => raw,
    }
}

/// Re-encodes a handshake with `server_port = 0`.
fn normalize_port(raw: &[u8]) -> Option<Vec<u8>> {
    let mut reader = protocol::PacketReader::new(raw);
    let mut handshake = protocol::Handshake::decode_payload(&mut reader).ok()?;
    if !reader.is_empty() {
        return None;
    }
    handshake.server_port = 0;
    handshake.encode_payload().ok()
}

/// Replaces the trailing 16 bytes of a `login_finished` payload (the
/// session UUID) with zeros.
///
/// The vanilla server draws a fresh random version-4 UUID per connection:
/// two recordings of the same scripted join share the first 41 bytes and
/// differ in the last 16, so the tail is per-connection randomness like
/// the handshake port, not a parity fact.
fn normalize_session_uuid(raw: &[u8]) -> Vec<u8> {
    if raw.len() < 16 {
        return raw.to_vec();
    }
    let mut out = raw.to_vec();
    let tail = out.len() - 16;
    out[tail..].fill(0);
    out
}

/// Replaces the first VarInt of `raw` (a teleport or entity id) with
/// [`NORMALIZED_ID`].
fn normalize_first_varint(raw: &[u8]) -> Vec<u8> {
    match protocol::decode_varint(raw) {
        Ok((_, consumed)) => {
            let mut out = Vec::with_capacity(raw.len() - consumed + 2);
            protocol::encode_varint(NORMALIZED_ID, &mut out);
            out.extend_from_slice(&raw[consumed..]);
            out
        }
        Err(_) => raw.to_vec(),
    }
}

/// Renders the owner-facing discrepancy report as Markdown (in Russian:
/// it is a document for the project owner, not code).
pub fn render_report(slice: &str, diff: &TracesDiff) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Парити-дифф среза `{slice}`: {} vs {}\n\n",
        diff.vanilla_target, diff.ours_target
    ));
    out.push_str("| Метрика | Значение |\n|---|---|\n");
    let _ = writeln!(out, "| Пакетов в трейсе ванили | {} |", diff.vanilla_total);
    let _ = writeln!(out, "| Пакетов в нашем трейсе | {} |", diff.ours_total);
    let _ = writeln!(
        out,
        "| Ключей с расхождением состава | {} |",
        diff.counts.len()
    );
    let _ = writeln!(
        out,
        "| Сопоставленных пакетов с расхождением payload | {} из {} |",
        diff.payloads.len(),
        diff.compared
    );
    let _ = writeln!(
        out,
        "| Итог | {} |",
        if diff.is_identical() {
            "трейсы совпадают"
        } else {
            "есть расхождения"
        }
    );

    if !diff.counts.is_empty() {
        out.push_str("\n## Расхождения по составу пакетов\n\n");
        out.push_str("| Состояние | Направление | Пакет | id | Ваниль | Наш | Δ |\n");
        out.push_str("|---|---|---|---|---|---|---|\n");
        for row in &diff.counts {
            let delta = row.ours as i64 - row.vanilla as i64;
            let _ = writeln!(
                out,
                "| {:?} | {} | {} | `0x{:02X}` | {} | {} | {:+} |",
                row.state,
                direction_name(row.direction),
                packet_label(row),
                row.id,
                row.vanilla,
                row.ours,
                delta
            );
        }
    }

    if !diff.payloads.is_empty() {
        out.push_str("\n## Расхождения payload (после нормализации случайностей)\n\n");
        out.push_str("| Пакет | Вхождение | Ваниль (hex, 48 байт) | Наш (hex, 48 байт) |\n");
        out.push_str("|---|---|---|---|\n");
        for row in &diff.payloads {
            let _ = writeln!(
                out,
                "| {} | #{} | `{}` | `{}` |",
                payload_label(row),
                row.occurrence,
                hex_prefix(&row.vanilla),
                hex_prefix(&row.ours)
            );
        }
    }
    out
}

/// Human direction name (the report is for the project owner).
fn direction_name(direction: TraceDirection) -> &'static str {
    match direction {
        TraceDirection::Clientbound => "к клиенту",
        TraceDirection::Serverbound => "к серверу",
    }
}

/// Markdown label of a count row: the packet name when known, else its
/// hex id.
fn packet_label(row: &CountRow) -> String {
    label(row.name.as_deref(), row.id)
}

/// Markdown label of a payload row: the packet name when known, else its
/// hex id.
fn payload_label(row: &PayloadRow) -> String {
    label(row.name.as_deref(), row.id)
}

/// The packet name in backticks, or the numeric id when the name is unknown.
fn label(name: Option<&str>, id: u32) -> String {
    match name {
        Some(name) => format!("`{name}`"),
        None => format!("`0x{id:02X}`"),
    }
}

/// First 48 bytes of `bytes` as hex (long payloads are cut for readability).
fn hex_prefix(bytes: &[u8]) -> String {
    let cut = &bytes[..bytes.len().min(48)];
    let mut text = to_hex(cut);
    if bytes.len() > 48 {
        text.push('…');
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a trace entry with a resolvable name for normalisation.
    fn entry(
        state: TraceState,
        direction: TraceDirection,
        id: u32,
        name: Option<&str>,
        payload: &[u8],
    ) -> TraceEntry {
        TraceEntry {
            state,
            direction,
            id,
            name: name.map(str::to_owned),
            payload: to_hex(payload),
        }
    }

    fn trace(target: &str, entries: Vec<TraceEntry>) -> Trace {
        Trace {
            target: target.to_owned(),
            packets: entries,
        }
    }

    #[test]
    fn count_differences_are_reported_per_packet_kind() {
        let vanilla = trace(
            "vanilla",
            vec![
                entry(
                    TraceState::Login,
                    TraceDirection::Clientbound,
                    0x03,
                    None,
                    &[1],
                ),
                entry(
                    TraceState::Login,
                    TraceDirection::Clientbound,
                    0x03,
                    None,
                    &[1],
                ),
            ],
        );
        let ours = trace(
            "ours",
            vec![entry(
                TraceState::Login,
                TraceDirection::Clientbound,
                0x03,
                None,
                &[1],
            )],
        );
        let diff = diff_traces(&vanilla, &ours);
        assert!(!diff.is_identical());
        assert_eq!(diff.counts.len(), 1);
        let row = &diff.counts[0];
        assert_eq!(row.vanilla, 2);
        assert_eq!(row.ours, 1);
        assert_eq!(row.id, 0x03);
        assert_eq!(diff.payloads.len(), 0);
        assert_eq!(diff.compared, 1);
    }

    #[test]
    fn handshake_port_difference_is_normalised_away() {
        let make = |port: u16| {
            trace(
                "server",
                vec![entry(
                    TraceState::Handshake,
                    TraceDirection::Serverbound,
                    0x00,
                    Some("minecraft:intention"),
                    &protocol::Handshake {
                        protocol_version: 777,
                        server_address: "localhost".to_owned(),
                        server_port: port,
                        next_state: protocol::NextState::Login,
                    }
                    .encode_payload()
                    .expect("handshake encodable"),
                )],
            )
        };
        let diff = diff_traces(&make(25566), &make(40000));
        assert!(diff.is_identical(), "port must not count: {diff:?}");
    }

    #[test]
    fn teleport_id_difference_is_normalised_but_coordinates_are_not() {
        let make = |teleport_id: i32, x: f64| {
            let payload = protocol::SynchronizePlayerPosition {
                teleport_id,
                x,
                y: 64.0,
                z: 0.0,
                velocity_x: 0.0,
                velocity_y: 0.0,
                velocity_z: 0.0,
                yaw: 0.0,
                pitch: 0.0,
                flags: 0,
            }
            .encode_payload();
            trace(
                "server",
                vec![entry(
                    TraceState::Play,
                    TraceDirection::Clientbound,
                    0x49,
                    Some("minecraft:player_position"),
                    &payload,
                )],
            )
        };
        let same_coords = diff_traces(&make(7, 1.0), &make(9, 1.0));
        assert!(same_coords.is_identical(), "teleport id must not count");
        let other_coords = diff_traces(&make(7, 1.0), &make(7, 2.0));
        assert_eq!(other_coords.payloads.len(), 1, "coordinates are real data");
    }

    /// Entity ids and teleport ids are allocated per connection: with the
    /// normalisation both sides of any recording collapse to one value, so
    /// only the payload's actual content is compared.
    #[test]
    fn per_connection_ids_collapse_to_one_value() {
        let make = |name: &str, first_id: i32, tail: &[u8]| {
            let mut payload = Vec::new();
            protocol::encode_varint(first_id, &mut payload);
            payload.extend_from_slice(tail);
            trace(
                "server",
                vec![entry(
                    TraceState::Play,
                    TraceDirection::Clientbound,
                    0x01,
                    Some(name),
                    &payload,
                )],
            )
        };
        // Same content, different ids as allocated by two different servers.
        let vanilla = make("minecraft:add_entity", 17, &[0x11, 0x22]);
        let ours = make("minecraft:add_entity", 42, &[0x11, 0x22]);
        assert!(
            diff_traces(&vanilla, &ours).is_identical(),
            "entity id must not count"
        );

        // The teleport id in either direction normalises the same way.
        let vanilla = make("minecraft:teleport_entity", 1, &[0x01]);
        let ours = make("minecraft:teleport_entity", 9, &[0x01]);
        assert!(
            diff_traces(&vanilla, &ours).is_identical(),
            "teleport id must not count"
        );

        // Real content behind the id is still compared.
        let vanilla = make("minecraft:add_entity", 17, &[0x11, 0x22]);
        let ours = make("minecraft:add_entity", 17, &[0x11, 0x23]);
        assert_eq!(
            diff_traces(&vanilla, &ours).payloads.len(),
            1,
            "payload content is real data"
        );
    }

    /// The random session UUID at the tail of login_finished carries no
    /// parity information; anything else in the payload still counts.
    #[test]
    fn login_finished_session_uuid_is_normalised() {
        let make = |tail: [u8; 16], prefix: &[u8]| {
            let mut payload = prefix.to_vec();
            payload.extend_from_slice(&tail);
            trace(
                "server",
                vec![entry(
                    TraceState::Login,
                    TraceDirection::Clientbound,
                    0x02,
                    Some("minecraft:login_finished"),
                    &payload,
                )],
            )
        };
        let prefix = [0xFF, 0x10, 0x70, 0xC9, 0xAA, 0xBB];
        let vanilla = make([0x4d; 16], &prefix);
        let ours = make([0x0a; 16], &prefix);
        assert!(
            diff_traces(&vanilla, &ours).is_identical(),
            "the random tail must not count"
        );

        // A difference before the tail (e.g. the profile UUID) still counts.
        let mut other = prefix.to_vec();
        other[5] = 0xCC;
        let ours = make([0x4d; 16], &other);
        assert_eq!(
            diff_traces(&vanilla, &ours).payloads.len(),
            1,
            "the payload prefix is real data"
        );
    }

    #[test]
    fn report_lists_both_tables() {
        let vanilla = trace(
            "vanilla",
            vec![entry(
                TraceState::Status,
                TraceDirection::Clientbound,
                0x00,
                Some("minecraft:status"),
                &[1],
            )],
        );
        let ours = trace("ours", Vec::new());
        let diff = diff_traces(&vanilla, &ours);
        let report = render_report("status", &diff);
        assert!(report.contains("## Расхождения по составу пакетов"));
        assert!(report.contains("`minecraft:status`"));
        assert!(report.contains("есть расхождения"));
        assert!(report.contains("| Пакетов в трейсе ванили | 1 |"));
    }
}
