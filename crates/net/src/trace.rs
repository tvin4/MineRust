//! Session packet trace of a live server — a diagnostic recording.
//!
//! The server writes every packet it sends and receives to a JSON file when
//! started with the `MINECRAFT_SESSION_TRACE` variable set. The flag makes
//! the live join of a real client comparable with the vanilla join recording
//! (`target/parity/join-vanilla.json`) packet by packet — order, names, ids,
//! sizes and timing — without changing anything about the protocol itself.
//!
//! Usage (PowerShell):
//!
//! ```text
//! $env:MINECRAFT_SESSION_TRACE = "1"        # -> target/parity/live-session.json
//! cargo run -p server                       # or = "<path>" for another file
//! ```
//!
//! File format: an opening `{"target":"live","started_ms":<unix ms>,"packets":[`,
//! one JSON record per packet, commas *between* the records (never after
//! the last one), then — on a clean process exit — the closing `]}`: a
//! cleanly closed file is valid JSON as it stands. A killed server leaves
//! the tail off, so readers close the document in memory (see the report
//! test of the parity crate).
//!
//! Writing runs on a plain thread fed by an unbounded channel: no `Mutex`
//! touches the connection tasks, a disabled trace costs one `OnceLock` read
//! per packet, and a full write is only ever appended behind the play loop.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tracing::{info, warn};

use data::PacketIds;
use protocol::{Direction, ProtocolState};

/// Environment variable that switches the session trace on.
///
/// `1`, `true`, `on` and an empty value select [`DEFAULT_PATH`]; any other
/// value is the path of the file to write; `0`, `false`, `off`, `no` keep
/// the trace off.
pub const TRACE_VAR: &str = "MINECRAFT_SESSION_TRACE";

/// File the trace lands in when the variable names no path of its own.
pub const DEFAULT_PATH: &str = "target/parity/live-session.json";

/// Values of [`TRACE_VAR`] that turn the trace off.
const OFF_VALUES: [&str; 4] = ["0", "off", "false", "no"];

/// Values of [`TRACE_VAR`] that mean "the default path, please".
const ON_VALUES: [&str; 3] = ["1", "true", "on"];

/// Payloads up to this many bytes are stored as hex; larger ones only get
/// their size (a chunk can be tens of kilobytes — the frame still proves
/// what went out, the coordinates are in its first eight bytes and logged).
const MAX_PAYLOAD: usize = 64 * 1024;

/// Reverse id → resource location, from the packets report when one is
/// loaded. Never fatal: an unnamed packet keeps its numeric id.
static NAMES: OnceLock<HashMap<(u8, u8, u32), String>> = OnceLock::new();

/// Channel to the writer thread; unset while tracing is off.
static SENDER: OnceLock<UnboundedSender<SessionEvent>> = OnceLock::new();

/// Trace start, the clock `t_ms` is measured against.
static ORIGIN: OnceLock<Instant> = OnceLock::new();

/// Order of the recorded packets (assigned before the event is queued).
static SEQ: AtomicU64 = AtomicU64::new(0);

/// One recorded packet: everything `live-vs-vanilla` needs to line two
/// sessions up against each other.
#[derive(Debug, Serialize)]
struct SessionEvent {
    /// Position of the packet in the recording (assigned by the server).
    seq: u64,
    /// Milliseconds since the trace was enabled.
    t_ms: f64,
    /// `serverbound` for packets the client sends, `clientbound` otherwise.
    dir: &'static str,
    /// Protocol state the packet belongs to (handshake/status/.../play).
    state: &'static str,
    /// Numeric packet id.
    id: u32,
    /// Resource location from the packets report, when known.
    name: Option<String>,
    /// Payload size in bytes (the id VarInt excluded).
    size: usize,
    /// Payload as hex; `null` above [`MAX_PAYLOAD`].
    payload: Option<String>,
}

/// Enable the session trace as [`TRACE_VAR`] asks for it.
///
/// Returns the file being written, or `None` when the variable is unset,
/// reads "off", or the file could not be opened (logged, never fatal: a
/// diagnostic flag must not stop the server).
pub fn start() -> Option<PathBuf> {
    let raw = std::env::var(TRACE_VAR).ok()?;
    let value = raw.trim();
    if OFF_VALUES.contains(&value) {
        info!(var = TRACE_VAR, "session trace disabled");
        return None;
    }
    let path = if value.is_empty() || ON_VALUES.contains(&value) {
        PathBuf::from(DEFAULT_PATH)
    } else {
        PathBuf::from(value)
    };
    if let Err(error) = init(&path) {
        warn!(path = %path.display(), %error, "cannot enable the session trace");
        return None;
    }
    info!(path = %path.display(), var = TRACE_VAR, "session trace enabled");
    Some(path)
}

/// Opens the file, writes its header and starts the writer thread.
///
/// Only the first call does anything: the channel is a process-wide
/// singleton, like the names it carries.
fn init(path: &Path) -> std::io::Result<()> {
    if SENDER.get().is_some() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let started_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or(0);
    let header = format!("{{\"target\":\"live\",\"started_ms\":{started_ms},\"packets\":[\n");
    let mut file = std::fs::File::create(path)?;
    file.write_all(header.as_bytes())?;
    file.flush()?;
    drop(file);

    let (sender, events) = unbounded_channel::<SessionEvent>();
    let writer_path = path.to_path_buf();
    std::thread::Builder::new()
        .name("session-trace".to_owned())
        .spawn(move || writer_loop(&writer_path, events))?;
    let _ = ORIGIN.set(Instant::now());
    let _ = SENDER.set(sender);
    Ok(())
}

/// Appends events to the file until the server drops every sender, then
/// closes the JSON document. Failures are logged once and end the thread;
/// the connection tasks never learn that tracing stopped working.
///
/// Commas separate the events *between* each other and are never written
/// after the last one, so the closed document parses as written — the
/// dangling comma of an interrupted writer is what the readers repair.
fn writer_loop(path: &Path, mut events: UnboundedReceiver<SessionEvent>) {
    let mut file = match std::fs::OpenOptions::new().append(true).open(path) {
        Ok(file) => file,
        Err(error) => {
            warn!(path = %path.display(), %error, "session trace writer stopped");
            while events.blocking_recv().is_some() {}
            return;
        }
    };
    let mut first = true;
    while let Some(event) = events.blocking_recv() {
        let line = match serde_json::to_string(&event) {
            Ok(line) => line,
            Err(error) => {
                warn!(%error, "cannot serialise a session trace packet");
                continue;
            }
        };
        let separator = if first {
            first = false;
            Ok(())
        } else {
            file.write_all(b",\n")
        };
        let written = separator
            .and_then(|()| file.write_all(line.as_bytes()))
            .and_then(|()| file.flush());
        if let Err(error) = written {
            warn!(path = %path.display(), %error, "session trace write failed");
            return;
        }
    }
    let _ = file.write_all(b"]}\n");
    let _ = file.flush();
}

/// Names to label recorded packets with: every id of the loaded report, or
/// the compiled packet keys when no report is configured.
pub fn names_for(ids: &PacketIds) -> HashMap<(u8, u8, u32), String> {
    if ids.is_empty() {
        return protocol::used_packet_keys()
            .iter()
            .map(|key| {
                (
                    (
                        key.state.index() as u8,
                        key.direction.index() as u8,
                        ids.id(*key),
                    ),
                    (*key.name).to_owned(),
                )
            })
            .collect();
    }
    ids.names_by_id()
}

/// Installs the id → resource location table used to label packets.
///
/// Called once at startup; a second call (a test racing the server) keeps
/// the names that are already installed.
pub fn install_names(names: HashMap<(u8, u8, u32), String>) {
    let _ = NAMES.set(names);
}

/// Records a packet the server writes to a connection.
pub fn sent(state: ProtocolState, id: u32, payload: &[u8]) {
    record(Direction::Clientbound, state, id, payload);
}

/// Records a packet the server reads from a connection.
pub fn received(state: ProtocolState, id: u32, payload: &[u8]) {
    record(Direction::Serverbound, state, id, payload);
}

/// Labels one packet, sends a debug log line (always, so `RUST_LOG=net=debug`
/// lists the packets in order with names, ids and sizes) and — when a trace
/// is running — queues the event for the writer thread.
fn record(direction: Direction, state: ProtocolState, id: u32, payload: &[u8]) {
    let name = NAMES
        .get()
        .and_then(|names| names.get(&(state.index() as u8, direction.index() as u8, id)))
        .cloned();
    let dir = direction.as_str();
    let message = match direction {
        Direction::Clientbound => "packet sent",
        Direction::Serverbound => "packet received",
    };
    tracing::debug!(
        dir,
        state = state.as_str(),
        id,
        name = name.as_deref().unwrap_or("<unnamed>"),
        size = payload.len(),
        "{message}"
    );

    let Some(sender) = SENDER.get() else {
        return;
    };
    let t_ms = ORIGIN
        .get()
        .map(|origin| origin.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0);
    let event = SessionEvent {
        seq: SEQ.fetch_add(1, Ordering::Relaxed),
        t_ms,
        dir,
        state: state.as_str(),
        id,
        name,
        size: payload.len(),
        payload: (payload.len() <= MAX_PAYLOAD).then(|| to_hex(payload)),
    };
    let _ = sender.send(event);
}

/// Encodes bytes as lowercase hex (the parity traces use the same digits).
fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0F)] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Labels come from the report; compiled mode falls back to the packet
    /// keys the code itself knows.
    #[test]
    fn compiled_ids_still_get_names() {
        let names = names_for(&PacketIds::compiled());
        let key = (
            ProtocolState::Play.index() as u8,
            Direction::Clientbound.index() as u8,
            0x2E,
        );
        assert_eq!(
            names.get(&key).map(String::as_str),
            Some("minecraft:level_chunk_with_light")
        );
    }

    #[test]
    fn hex_uses_the_parity_digits() {
        assert_eq!(to_hex(&[0x00, 0x0F, 0xAB, 0xFF]), "000fabff");
    }

    /// Packets reach the file: a header, then one JSON line per packet.
    #[tokio::test]
    async fn a_trace_file_records_every_packet() {
        let path = std::env::temp_dir().join(format!(
            "minerust-session-trace-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        init(&path).expect("trace file opens");

        sent(ProtocolState::Play, 0x2E, &[1, 2, 3]);
        received(ProtocolState::Play, 0x1C, &[]);

        let mut text = String::new();
        for _ in 0..100 {
            text = std::fs::read_to_string(&path).unwrap_or_default();
            if text.contains("\"size\":0") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(
            text.starts_with("{\"target\":\"live\",\"started_ms\":"),
            "{text}"
        );
        assert!(text.contains("\"dir\":\"clientbound\""), "{text}");
        assert!(text.contains("\"dir\":\"serverbound\""), "{text}");
        assert!(text.contains("\"state\":\"play\""), "{text}");
        assert!(text.contains("\"size\":3"), "{text}");
        assert!(text.contains("\"payload\":\"010203\""), "{text}");
        let _ = std::fs::remove_file(&path);
    }

    /// The closing bracket follows the drop of the last sender — the shape
    /// the report loader expects, exercised here on a channel of its own
    /// (the process-wide one outlives any single test).
    #[test]
    fn the_writer_closes_the_document_when_the_channel_ends() {
        let path = std::env::temp_dir().join(format!(
            "minerust-session-trace-close-{}.json",
            std::process::id()
        ));
        std::fs::write(&path, "{\"target\":\"live\",\"packets\":[\n").expect("header");
        let (sender, events) = unbounded_channel::<SessionEvent>();
        sender
            .send(SessionEvent {
                seq: 0,
                t_ms: 1.5,
                dir: "clientbound",
                state: "play",
                id: 7,
                name: None,
                size: 0,
                payload: None,
            })
            .expect("queued");
        drop(sender);

        writer_loop(&path, events);
        let text = std::fs::read_to_string(&path).expect("trace file");
        assert!(text.ends_with("]}\n"), "{text}");
        assert!(text.contains("\"t_ms\":1.5"), "{text}");
        // Both the events and the tail together form valid JSON.
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(parsed["packets"][0]["id"], 7);
        let _ = std::fs::remove_file(&path);
    }
}
