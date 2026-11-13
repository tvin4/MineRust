//! Raw packet traces recorded by the parity test client.
//!
//! A trace is the wire truth: for every packet the client sent or received it
//! stores `(state, direction, id, payload)` with the payload as raw hex
//! bytes. Field-level parsing is only ever used later, for normalising
//! randomness (player name, UUIDs, random ids) before a diff — never as a
//! substitute for comparing the bytes.

use std::path::Path;

use protocol::{Direction, ProtocolState};
use serde::{Deserialize, Serialize};

/// Protocol state a recorded packet belongs to (serialisable form of
/// [`ProtocolState`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceState {
    /// The initial handshake.
    Handshake,
    /// Server list ping.
    Status,
    /// Login sequence.
    Login,
    /// Configuration exchange.
    Configuration,
    /// The play state.
    Play,
}

impl From<ProtocolState> for TraceState {
    fn from(state: ProtocolState) -> Self {
        match state {
            ProtocolState::Handshake => Self::Handshake,
            ProtocolState::Status => Self::Status,
            ProtocolState::Login => Self::Login,
            ProtocolState::Configuration => Self::Configuration,
            ProtocolState::Play => Self::Play,
        }
    }
}

/// Direction of a recorded packet (serialisable form of [`Direction`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceDirection {
    /// Sent by the server.
    Clientbound,
    /// Sent by the client.
    Serverbound,
}

impl From<Direction> for TraceDirection {
    fn from(direction: Direction) -> Self {
        match direction {
            Direction::Clientbound => Self::Clientbound,
            Direction::Serverbound => Self::Serverbound,
        }
    }
}

/// One packet as seen on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceEntry {
    /// Protocol state at the moment the packet was sent or received.
    pub state: TraceState,
    /// Direction relative to the server.
    pub direction: TraceDirection,
    /// Numeric packet id on the wire.
    pub id: u32,
    /// Packet name from the packets report, when the id resolves to one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Payload bytes (everything after the packet id) as lowercase hex.
    pub payload: String,
}

/// The full recording of one connection script against one server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trace {
    /// Which side produced the trace, e.g. `vanilla` or `ours`.
    pub target: String,
    /// Packets in the order they were sent or received.
    pub packets: Vec<TraceEntry>,
}

impl Trace {
    /// Creates an empty trace for a target server.
    pub fn new(target: &str) -> Self {
        Self {
            target: target.to_owned(),
            packets: Vec::new(),
        }
    }

    /// Appends one raw packet.
    pub fn record(&mut self, state: ProtocolState, direction: Direction, id: u32, payload: &[u8]) {
        self.packets.push(TraceEntry {
            state: state.into(),
            direction: direction.into(),
            id,
            name: None,
            payload: to_hex(payload),
        });
    }

    /// Number of recorded packets.
    pub fn len(&self) -> usize {
        self.packets.len()
    }

    /// True when nothing was recorded (always a test failure).
    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    /// Serialises the trace as pretty-printed JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Writes the JSON form to a file, creating parent directories.
    pub fn save(&self, path: &Path) -> Result<(), TraceIoError> {
        let display = path.display().to_string();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| TraceIoError::Io {
                path: display.clone(),
                source,
            })?;
        }
        std::fs::write(path, self.to_json()?).map_err(|source| TraceIoError::Io {
            path: display,
            source,
        })
    }

    /// Reads a trace back from its JSON form.
    pub fn load(path: &Path) -> Result<Self, TraceIoError> {
        let display = path.display().to_string();
        let text = std::fs::read_to_string(path).map_err(|source| TraceIoError::Io {
            path: display,
            source,
        })?;
        Ok(serde_json::from_str(&text)?)
    }
}

/// Errors of writing or reading a trace file.
#[derive(Debug, thiserror::Error)]
pub enum TraceIoError {
    /// The file itself could not be read or written.
    #[error("trace file {path}: {source}")]
    Io {
        /// File involved.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The JSON representation is invalid.
    #[error("invalid trace json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Encodes bytes as lowercase hex.
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(DIGITS[usize::from(byte >> 4)] as char);
        out.push(DIGITS[usize::from(byte & 0x0F)] as char);
    }
    out
}

/// Decodes a lowercase hex string back into bytes.
pub fn from_hex(text: &str) -> Result<Vec<u8>, HexError> {
    if !text.len().is_multiple_of(2) {
        return Err(HexError::OddLength(text.len()));
    }
    let digits = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    let (pairs, _) = digits.as_chunks::<2>();
    for pair in pairs {
        let hi = hex_digit(pair[0])?;
        let lo = hex_digit(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

/// Decodes standard base64 with padding — the JSONL payload encoding written
/// by [`crate::walk::Jsonl`] — back into raw bytes.
///
/// Returns `None` on any character outside the alphabet or a character after
/// padding: a broken line is a reading failure, never guessed bytes.
pub fn from_base64(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let mut padded = false;
    for byte in text.bytes() {
        if byte == b'=' {
            padded = true;
            continue;
        }
        if padded {
            return None;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Failure of [`from_hex`].
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HexError {
    /// The string has a dangling half-byte.
    #[error("odd hex string length {0}")]
    OddLength(usize),
    /// A character is not a hex digit.
    #[error("invalid hex digit 0x{0:02x}")]
    InvalidDigit(u8),
}

fn hex_digit(byte: u8) -> Result<u8, HexError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        other => Err(HexError::InvalidDigit(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrips() {
        let bytes = [0x00, 0x01, 0xAB, 0xFF, 0x7F];
        let text = to_hex(&bytes);
        assert_eq!(text, "0001abff7f");
        assert_eq!(from_hex(&text).expect("valid hex"), bytes);
    }

    #[test]
    fn from_hex_rejects_odd_and_invalid() {
        assert!(matches!(from_hex("abc"), Err(HexError::OddLength(3))));
        assert!(from_hex("zz").is_err());
    }

    #[test]
    fn base64_decodes_the_jsonl_payload_encoding() {
        assert_eq!(from_base64(""), Some(Vec::new()));
        assert_eq!(from_base64("AA=="), Some(vec![0x00]));
        assert_eq!(from_base64("AAEC"), Some(vec![0x00, 0x01, 0x02]));
        assert_eq!(from_base64("4dB/"), Some(vec![0xE1, 0xD0, 0x7F]));
        assert_eq!(from_base64("zz!!"), None);
        assert_eq!(from_base64("AA=A"), None);
    }

    #[test]
    fn trace_serialises_packets_in_order() {
        let mut trace = Trace::new("vanilla");
        trace.record(ProtocolState::Status, Direction::Clientbound, 0x00, &[1, 2]);
        trace.record(ProtocolState::Status, Direction::Serverbound, 0x01, &[]);
        let json = trace.to_json().expect("json");
        let loaded = serde_json::from_str::<Trace>(&json).expect("parse");
        assert_eq!(loaded, trace);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.packets[0].state, TraceState::Status);
        assert_eq!(loaded.packets[1].direction, TraceDirection::Serverbound);
    }
}
