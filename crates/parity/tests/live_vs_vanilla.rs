//! Step 4 of the live check: the recorded session vs the vanilla join
//! recording — a report, never an assertion.
//!
//! The session to compare is picked in this order:
//!
//! 1. `MINECRAFT_SESSION_FILE` (explicit path of any recording), else
//! 2. `target/parity/live-session.json` (the server's own trace of a real
//!    client session, `MINECRAFT_SESSION_TRACE=1`), else
//! 3. `target/parity/self-session.json` (our in-process server joined by
//!    the scripted client, `self_session.rs`), else
//! 4. `target/parity/join-ours.json` (the recorded join slice — good for a
//!    first, pause-free comparison).
//!
//! Both file formats are understood: the parity trace the scripted client
//! writes, and the server's own live format (`dir` instead of `direction`,
//! an optional `payload` above the trace's cap, a tail left open or
//! dangling when the server was killed — `salvage_tail` closes it in
//! memory). The report lands in
//! `target/parity-report/live-vs-vanilla.md` and answers the questions of
//! the live check: first packets in order, only-vanilla / only-ours, the
//! serverbound line, the chunk cache centre and the chunk coordinates
//! against the position (96.5, 145, -31.5).
//!
//! Nothing here fails on a discrepancy: the owner reads the report and
//! decides. Missing files or an unreadable format are a printed SKIP.

use std::path::{Path, PathBuf};

use parity::{diff_traces, from_hex, render_report, Trace, TraceDirection, TraceEntry, TraceState};

/// Explicit override of which session recording to compare.
const SESSION_FILE_VAR: &str = "MINECRAFT_SESSION_FILE";

/// The chunk a join at (96.5, 145, -31.5) must be served under.
const EXPECTED_CHUNK: (i32, i32) = (6, -2);

/// Compiled packet id of `minecraft:level_chunk_with_light`, the fallback
/// when a recording carries no names (the report's id is 46 = 0x2E and the
/// startup verifies the two agree).
const CHUNK_PACKET_ID: u32 = 0x2E;

/// View distance the server announces in Login (play): the radius of the
/// square around the centre every allowed chunk must fall in.
// SOURCE: our own `net` join constant, documented in server.example.toml.
const VIEW_DISTANCE: i32 = 10;

/// Where recordings land: `<workspace>/target/parity/`.
fn trace_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("parity")
}

/// Where reports land: `<workspace>/target/parity-report/`.
fn report_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("parity-report")
}

/// The session file to compare: env override, then the variants by
/// priority, first existing wins.
fn session_path() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(explicit) = std::env::var(SESSION_FILE_VAR) {
        if !explicit.trim().is_empty() {
            candidates.push(PathBuf::from(explicit));
        }
    }
    candidates.push(trace_dir().join("live-session.json"));
    candidates.push(trace_dir().join("self-session.json"));
    candidates.push(trace_dir().join("join-ours.json"));
    candidates.into_iter().find(|path| path.exists())
}

/// Loads either recording format: the parity trace first, the server's own
/// live format (with its open tail salvaged) otherwise.
fn load_session(path: &Path) -> Result<Trace, String> {
    if let Ok(trace) = Trace::load(path) {
        return Ok(trace);
    }
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    live_session_to_trace(&text).map_err(|error| format!("{}: {error}", path.display()))
}

/// Converts the server's live session JSON into a parity trace record by
/// record (`dir` → direction, `payload: null` stays as no payload).
fn live_session_to_trace(text: &str) -> Result<Trace, String> {
    let document: serde_json::Value = serde_json::from_str(&salvage_tail(text))
        .map_err(|error| format!("not JSON even after salvaging the tail: {error}"))?;
    let packets = document
        .get("packets")
        .and_then(serde_json::Value::as_array)
        .ok_or("no packets array")?;
    let target = document
        .get("target")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("live")
        .to_owned();
    let mut out = Trace::new(&target);
    for packet in packets {
        out.packets.push(live_entry(packet)?);
    }
    Ok(out)
}

/// One live-format packet as a parity [`TraceEntry`].
fn live_entry(packet: &serde_json::Value) -> Result<TraceEntry, String> {
    let state: TraceState = serde_json::from_value(
        packet
            .get("state")
            .cloned()
            .ok_or_else(|| "packet without state".to_owned())?,
    )
    .map_err(|error| format!("unknown state: {error}"))?;
    let direction: TraceDirection = serde_json::from_value(
        packet
            .get("dir")
            .cloned()
            .ok_or_else(|| "packet without dir".to_owned())?,
    )
    .map_err(|error| format!("unknown direction: {error}"))?;
    let id = packet
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .ok_or("packet without a numeric id")? as u32;
    let name = packet
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    // Payloads above the trace's cap are recorded as `null`: keep the
    // absence, the payload content is not what this comparison is for.
    let payload = packet
        .get("payload")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    Ok(TraceEntry {
        state,
        direction,
        id,
        name,
        payload,
    })
}

/// Closes the document of the live server in memory — the file on disk
/// stays untouched.
///
/// A killed process never writes the closing `]}` at all, and an
/// interrupted writer of an older build can leave a dangling comma before
/// the end; both shapes are repaired here, the comma dropped right before
/// the bracket.
fn salvage_tail(text: &str) -> String {
    let mut repaired = text.trim_end().to_owned();
    if !repaired.ends_with(']') && !repaired.ends_with("]}") {
        if !repaired.ends_with(',') {
            repaired.push(',');
        }
        repaired.push_str("\n]}\n");
    }
    if let Some(close) = repaired.rfind(']') {
        let head = repaired[..close].trim_end();
        if head.ends_with(',') {
            let tail: String = repaired[close..].chars().collect();
            // Cut the comma (and the whitespace between it and the bracket).
            repaired.truncate(head.len() - 1);
            repaired.push('\n');
            repaired.push_str(&tail);
        }
    }
    repaired
}

/// The join window: from the first login-state packet of the recording up
/// to and including the first chunk (or the end, when none arrived).
fn join_window(trace: &Trace) -> Vec<&TraceEntry> {
    if trace.packets.is_empty() {
        return Vec::new();
    }
    let start = trace
        .packets
        .iter()
        .position(|entry| entry.state == TraceState::Login)
        .unwrap_or(0);
    let end = trace.packets[start..]
        .iter()
        .position(is_chunk)
        .map(|offset| start + offset)
        .unwrap_or(trace.packets.len() - 1);
    trace.packets[start..=end].iter().collect()
}

/// Whether this entry is the client's chunk packet (by name, or by the
/// compiled id when a recording carries no names).
fn is_chunk(entry: &TraceEntry) -> bool {
    entry.direction == TraceDirection::Clientbound
        && match entry.name.as_deref() {
            Some(name) => name == "minecraft:level_chunk_with_light",
            None => entry.id == CHUNK_PACKET_ID,
        }
}

/// Payload bytes of an entry (empty when the server recorded only size).
fn payload(entry: &TraceEntry) -> Vec<u8> {
    from_hex(&entry.payload).unwrap_or_default()
}

/// Chunk coordinates as the first two big-endian i32 of the payload, in
/// record order.
fn chunks(trace: &Trace) -> Vec<(i32, i32)> {
    trace
        .packets
        .iter()
        .filter(|entry| is_chunk(entry))
        .filter_map(|entry| {
            let bytes = payload(entry);
            let (x, rest) = bytes.split_first_chunk::<4>()?;
            let (z, _) = rest.split_first_chunk::<4>()?;
            Some((i32::from_be_bytes(*x), i32::from_be_bytes(*z)))
        })
        .collect()
}

/// The chunk cache centre from `set_chunk_cache_center`: two VarInts.
fn center(trace: &Trace) -> Option<(i32, i32)> {
    trace
        .packets
        .iter()
        .find(|entry| {
            entry.name.as_deref() == Some("minecraft:set_chunk_cache_center")
                && entry.direction == TraceDirection::Clientbound
        })
        .and_then(|entry| {
            let bytes = payload(entry);
            let (x, used) = protocol::decode_varint(&bytes).ok()?;
            let (z, _) = protocol::decode_varint(&bytes[used..]).ok()?;
            Some((x, z))
        })
}

/// Human packet label of one record: `state direction name id size`.
fn label(entry: &TraceEntry) -> String {
    let size = entry.payload.len() / 2;
    let name = entry
        .name
        .as_deref()
        .map(|name| format!("`{name}`"))
        .unwrap_or_else(|| format!("`0x{:02X}`", entry.id));
    format!(
        "{:?} {:?} {name} id={} bytes={size}",
        entry.state, entry.direction, entry.id
    )
}

/// Whether both records are the same packet at the same place (the size is
/// deliberately off the list: a live payload above the trace's cap is
/// recorded without bytes and would fake a mismatch).
fn same_packet(left: &TraceEntry, right: &TraceEntry) -> bool {
    left.state == right.state
        && left.direction == right.direction
        && left.id == right.id
        && left.name == right.name
}

/// Section one: the ordered packets from login to the first chunk, two
/// columns side by side, mismatched positions marked.
fn order_section(vanilla: &Trace, session: &Trace) -> String {
    let left = join_window(vanilla);
    let right = join_window(session);
    let rows = left.len().max(right.len());
    let mut out = String::from(
        "\n## Порядок пакетов: от login до первого чанка\n\n\
         | # | Ваниль | Наш сеанс | Совпало |\n|---|---|---|---|\n",
    );
    for index in 0..rows {
        let vn = left.get(index);
        let sn = right.get(index);
        let cell = |entry: Option<&&TraceEntry>| {
            entry
                .map(|entry| label(entry))
                .unwrap_or_else(|| "—".to_owned())
        };
        let same = match (vn, sn) {
            (Some(v), Some(s)) => same_packet(v, s),
            _ => false,
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            index + 1,
            cell(vn),
            cell(sn),
            if same { "✓" } else { "**✗**" }
        ));
    }
    out
}

/// Section: everything the client sent back to the server, in read order.
fn serverbound_section(vanilla: &Trace, session: &Trace) -> String {
    fn render(trace: &Trace) -> String {
        trace
            .packets
            .iter()
            .filter(|entry| entry.direction == TraceDirection::Serverbound)
            .map(label)
            .map(|line| format!("- {line}\n"))
            .collect()
    }
    format!(
        "\n## Serverbound в порядке чтения\n\n### Ваниль\n{}\n### Наш сеанс\n{}",
        render(vanilla),
        render(session)
    )
}

/// Section: centre and chunk coordinates against the join position, with
/// ✓/✗ verdicts — reported, never asserted.
///
/// Chunk coordinates live in the first eight payload bytes, so a packet
/// recorded over the trace's payload cap (a null payload) can be counted
/// but not placed: such rows say «не проверяемо» instead of a fake ✗.
fn chunks_section(name: &str, trace: &Trace) -> String {
    let packets: Vec<&TraceEntry> = trace
        .packets
        .iter()
        .filter(|entry| is_chunk(entry))
        .collect();
    let found = chunks(trace);
    let center = center(trace);
    let readable = !found.is_empty() && found.len() == packets.len();
    let outside: Vec<(i32, i32)> = match center {
        Some((cx, cz)) => found
            .iter()
            .copied()
            .filter(|(x, z)| (x - cx).abs() > VIEW_DISTANCE || (z - cz).abs() > VIEW_DISTANCE)
            .collect(),
        None => Vec::new(),
    };
    let unavailable = "⚠ не проверяемо: payload не записан (> cap трейса)";
    let (at_expected, outside_row) = if packets.is_empty() {
        ("— пакетов нет".to_owned(), "— пакетов нет".to_owned())
    } else if !readable {
        (unavailable.to_owned(), unavailable.to_owned())
    } else {
        let expected = if found.contains(&EXPECTED_CHUNK) {
            "✓ есть".to_owned()
        } else {
            "**✗ нет**".to_owned()
        };
        let outside_row = if outside.is_empty() {
            "✓ нет".to_owned()
        } else {
            format!("**✗ {}**: {outside:?}", outside.len())
        };
        (expected, outside_row)
    };
    let list = found
        .iter()
        .map(|(x, z)| format!("({x}, {z})"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = format!("\n## Чанки: {name}\n\n");
    out.push_str(&format!(
        "| Проверка | Значение |\n|---|---|\n\
         | Пакетов `level_chunk_with_light` | {} |\n\
         | Координат извлечено (из первых 8 байт payload) | {} из {} |\n\
         | Центр `set_chunk_cache_center` (два VarInt) | {center:?} (ожидаем {EXPECTED_CHUNK:?}) |\n\
         | Чанк {EXPECTED_CHUNK:?} под позицией (96.5, 145, -31.5) | {at_expected} |\n\
         | Чанков вне центр±{VIEW_DISTANCE} | {outside_row} |\n",
        packets.len(),
        found.len(),
        packets.len(),
    ));
    if !found.is_empty() {
        out.push_str(&format!("\nКоординаты в порядке отправки: {list}\n"));
    }
    out
}

/// The whole owner-facing report: the diff tables first, then the sections
/// the live check asks for.
fn build_report(vanilla: &Trace, session: &Trace, path: &Path) -> String {
    let diff = diff_traces(vanilla, session);
    let mut out = format!(
        "Сравнение сессии `{}` ({} пакетов) с ванильным join `{}` ({} пакетов), вход: `{}`.\n",
        session.target,
        session.len(),
        vanilla.target,
        vanilla.len(),
        path.display()
    );
    out.push_str(&render_report("live-vs-vanilla", &diff));
    out.push_str(&order_section(vanilla, session));
    out.push_str(&serverbound_section(vanilla, session));
    out.push_str(&chunks_section("ваниль", vanilla));
    out.push_str(&chunks_section("наш сеанс", session));
    out
}

#[test]
fn the_session_is_reported_against_the_vanilla_join() {
    let vanilla_path = trace_dir().join("join-vanilla.json");
    let Some(session_path) = session_path() else {
        eprintln!(
            "SKIP: no session recording found; set {SESSION_FILE_VAR} or record one \
             (MINECRAFT_SESSION_TRACE=1, self_session.rs, join_slice.rs)"
        );
        return;
    };
    if !vanilla_path.exists() {
        eprintln!(
            "SKIP: {} missing; run the join slice first (join_slice.rs --ignored)",
            vanilla_path.display()
        );
        return;
    }
    let vanilla = match Trace::load(&vanilla_path) {
        Ok(trace) => trace,
        Err(error) => {
            eprintln!("SKIP: cannot read {}: {error}", vanilla_path.display());
            return;
        }
    };
    let session = match load_session(&session_path) {
        Ok(trace) => trace,
        Err(error) => {
            eprintln!("SKIP: cannot read the session: {error}");
            return;
        }
    };

    let report = build_report(&vanilla, &session, &session_path);
    let out = report_dir().join("live-vs-vanilla.md");
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("report dir");
    }
    std::fs::write(&out, &report).expect("report written");
    println!("{report}");
    println!("report written to {}", out.display());
}

#[cfg(test)]
mod salvage {
    /// A killed server: last event with its comma, no closing bracket.
    #[test]
    fn an_open_tail_becomes_valid_json() {
        let text = "{\n\"packets\":[\n{\"id\":1},\n";
        let repaired = super::salvage_tail(text);
        let parsed: serde_json::Value = serde_json::from_str(&repaired).expect("valid json");
        assert_eq!(parsed["packets"][0]["id"], 1);
    }

    /// A clean close still carries the dangling comma of the writer.
    #[test]
    fn a_closed_tail_keeps_its_last_event() {
        let text = "{\"packets\":[\n{\"id\":1},\n{\"id\":2},\n]}\n";
        let repaired = super::salvage_tail(text);
        let parsed: serde_json::Value = serde_json::from_str(&repaired).expect("valid json");
        assert_eq!(parsed["packets"].as_array().map(Vec::len), Some(2));
        assert_eq!(parsed["packets"][1]["id"], 2);
    }
}
