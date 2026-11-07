//! Loading of numeric packet ids from the data generator's packets report.
//!
//! The data generator of the game (`--reports`) writes `reports/packets.json`:
//! for every state of the protocol, for every direction, the resource
//! location of each packet mapped to its numeric `protocol_id`. That report
//! is the authoritative source of packet ids of a particular game version.
//!
//! The report is produced outside the repository (like registry data it
//! belongs to the user's own copy of the game) and pointed at by the
//! optional `packets_report` setting of the server configuration. Without a
//! report the server falls back to the compiled defaults of the
//! [`PacketKey`]s, which are documented but not verified against the game.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::error::DataError;
use protocol::{Direction, PacketKey, ProtocolState};

/// Numeric id of one packet entry of the report.
#[derive(Debug, Deserialize)]
struct ProtocolId {
    /// Id as it appears in the report.
    protocol_id: u32,
}

/// The packets of one state, grouped by direction.
///
/// The handshake state has no clientbound packets, so the report omits that
/// key entirely; both directions therefore default to empty.
#[derive(Debug, Default, Deserialize)]
struct StateReport {
    /// Packets the server sends in this state.
    #[serde(default)]
    clientbound: HashMap<String, ProtocolId>,
    /// Packets the client sends in this state.
    #[serde(default)]
    serverbound: HashMap<String, ProtocolId>,
}

/// Root of `reports/packets.json`: one entry per protocol state.
///
/// A state the report does not contain is rejected explicitly; extra fields
/// of a future report format are ignored.
#[derive(Debug, Deserialize)]
struct ReportFile {
    #[serde(default)]
    handshake: Option<StateReport>,
    #[serde(default)]
    status: Option<StateReport>,
    #[serde(default)]
    login: Option<StateReport>,
    #[serde(default)]
    configuration: Option<StateReport>,
    #[serde(default)]
    play: Option<StateReport>,
}

/// Numeric packet ids resolved by [`PacketKey`], in every protocol state.
///
/// Without a loaded report ([`PacketIds::compiled`]) every key resolves to
/// its compiled default id; with a report loaded the report's id wins, so
/// the ids the server speaks are taken from the game's own data.
#[derive(Debug)]
pub struct PacketIds {
    /// Per state, per direction, id of each packet by resource location.
    states: [StateIds; ProtocolState::COUNT],
}

/// Packet ids of a single state.
#[derive(Debug, Default)]
struct StateIds {
    /// Ids of the two directions, indexed by [`Direction::index`].
    directions: [HashMap<String, u32>; Direction::COUNT],
}

impl PacketIds {
    /// Ids that come solely from the compiled defaults of the keys.
    ///
    /// This is the fallback for a server started without a report; every id
    /// resolves to `key.default_id`.
    pub fn compiled() -> Self {
        Self {
            states: std::array::from_fn(|_| StateIds::default()),
        }
    }

    /// Number of ids loaded from a report (compiled mode has none).
    pub fn len(&self) -> usize {
        self.states
            .iter()
            .flat_map(|state| state.directions.iter())
            .map(HashMap::len)
            .sum()
    }

    /// Whether no report was loaded (compiled mode).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The report's id for `key`, if that id was loaded.
    fn lookup(&self, key: PacketKey) -> Option<u32> {
        let state = &self.states[key.state.index()];
        state.directions[key.direction.index()]
            .get(key.name)
            .copied()
    }

    /// Numeric id of `key`: the report's id when loaded, otherwise the
    /// compiled default of the key.
    pub fn id(&self, key: PacketKey) -> u32 {
        self.lookup(key).unwrap_or(key.default_id)
    }

    /// Checks that the loaded report contains every key.
    ///
    /// In compiled mode (no report) every key is missing, so this is only
    /// meaningful after loading a report; a missing key means the report
    /// belongs to a different game version than the server expects.
    pub fn verify(&self, keys: &[PacketKey]) -> Result<(), DataError> {
        let missing: Vec<String> = keys
            .iter()
            .filter(|key| self.lookup(**key).is_none())
            .map(PacketKey::describe)
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        Err(DataError::MissingPackets {
            count: missing.len(),
            keys: missing.join(", "),
        })
    }
}

/// Loads `reports/packets.json` from `path` (the data generator's report).
pub fn load_packets_report(path: impl AsRef<Path>) -> Result<PacketIds, DataError> {
    let path_ref = path.as_ref();
    let display = path_ref.display().to_string();
    let text = fs::read_to_string(path_ref).map_err(|source| DataError::ReportRead {
        path: display.clone(),
        source,
    })?;
    parse_report(&text, display)
}

/// Parses the JSON text of a packets report into [`PacketIds`].
pub fn packets_from_json(text: &str) -> Result<PacketIds, DataError> {
    parse_report(text, "packets report".to_owned())
}

/// Shared parser: turns the report text into ids, naming `path` in errors.
fn parse_report(text: &str, path: String) -> Result<PacketIds, DataError> {
    let file: ReportFile =
        serde_json::from_str(text).map_err(|source| DataError::Json { path, source })?;
    let mut ids = PacketIds::compiled();
    let reports = [
        (ProtocolState::Handshake, file.handshake),
        (ProtocolState::Status, file.status),
        (ProtocolState::Login, file.login),
        (ProtocolState::Configuration, file.configuration),
        (ProtocolState::Play, file.play),
    ];
    for (state, report) in reports {
        let Some(report) = report else {
            return Err(DataError::MissingState {
                state: state.as_str().to_owned(),
            });
        };
        let directions = &mut ids.states[state.index()].directions;
        for (name, id) in report.clientbound {
            directions[Direction::Clientbound.index()].insert(name, id.protocol_id);
        }
        for (name, id) in report.serverbound {
            directions[Direction::Serverbound.index()].insert(name, id.protocol_id);
        }
    }
    Ok(ids)
}
