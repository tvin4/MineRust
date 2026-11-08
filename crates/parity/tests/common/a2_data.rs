//! Data helpers for the Part A2 evidence test: payload walking without a
//! full decode, wire container expansion and the reverse block-state table.
//!
//! Included by path from `tests/a2_findings.rs` (a file in `tests/common`
//! is not a target of its own). Everything here reads recorded bytes or
//! the game's own `blocks.json` report — see `docs/DECISIONS.md`, part A2.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;

use protocol::{ChunkSection, LevelChunkWithLight, PaletteContainer};

/// Hex payload bytes decoded to raw bytes.
pub fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex"))
        .collect()
}

/// All `level_chunk_with_light` payloads of a recorded trace file, in
/// trace order, with coordinates (the first eight payload bytes).
pub fn payloads_of(trace: &Path) -> Vec<((i32, i32), Vec<u8>)> {
    let text =
        std::fs::read_to_string(trace).unwrap_or_else(|e| panic!("{}: {e}", trace.display()));
    let value: serde_json::Value = serde_json::from_str(&text).expect("trace JSON");
    let mut out = Vec::new();
    for packet in value["packets"].as_array().expect("packets") {
        if packet["name"] != "minecraft:level_chunk_with_light" {
            continue;
        }
        let bytes = hex_to_bytes(packet["payload"].as_str().expect("hex payload"));
        if bytes.len() < 8 {
            continue;
        }
        let x = i32::from_be_bytes(bytes[0..4].try_into().expect("x"));
        let z = i32::from_be_bytes(bytes[4..8].try_into().expect("z"));
        out.push(((x, z), bytes));
    }
    out
}

/// One payload decoded; the recording must be fully readable (the same
/// strictness as `chunk_slice`).
pub fn decode(coords: (i32, i32), bytes: &[u8]) -> LevelChunkWithLight {
    let mut reader = protocol::PacketReader::new(bytes);
    let chunk = LevelChunkWithLight::decode_payload(&mut reader)
        .unwrap_or_else(|e| panic!("chunk ({}, {}) decodable: {e}", coords.0, coords.1));
    reader
        .ensure_empty()
        .unwrap_or_else(|e| panic!("chunk ({}, {}) fully read: {e}", coords.0, coords.1));
    chunk
}

/// A VarInt read from `bytes` at `at`; `None` when it runs past the end.
fn read_varint(bytes: &[u8], at: usize) -> Option<(i32, usize)> {
    let mut value = 0_u32;
    let mut taken = 0_usize;
    loop {
        let byte = *bytes.get(at + taken)?;
        value |= u32::from(byte & 0x7f) << (taken * 7);
        taken += 1;
        if byte & 0x80 == 0 {
            return Some((value as i32, taken));
        }
        if taken == 5 {
            return None;
        }
    }
}

/// Heightmap id order of one payload, read off the header without decoding
/// the sections behind it.
pub fn heightmap_order(bytes: &[u8]) -> Vec<i32> {
    let count = read_varint(bytes, 8).map(|(n, _)| n).unwrap_or(0);
    let mut at = 8 + read_varint(bytes, 8).map(|(_, len)| len).unwrap_or(0);
    let mut ids = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let (id, len) = read_varint(bytes, at).expect("heightmap id");
        at += len;
        let (words, len) = read_varint(bytes, at).expect("heightmap words");
        at += len;
        at += words as usize * 8;
        ids.push(id);
    }
    ids
}

/// Expands one wire container into its 4096 (or 64) cell values: state or
/// biome ids. Packing matches `data_len` of the protocol crate — one word
/// holds `64 / bits` cells, big-endian.
pub fn cells(container: &PaletteContainer, count: usize) -> Option<Vec<i32>> {
    match container {
        PaletteContainer::Single(value) => Some(vec![*value; count]),
        PaletteContainer::Packed {
            bits,
            palette,
            data,
        } => {
            let bits = *bits as usize;
            let per_word = 64 / bits;
            let mask = (1_u64 << bits) - 1;
            let mut out = Vec::with_capacity(count);
            for cell in 0..count {
                let word = cell / per_word;
                let offset = (cell % per_word) * bits;
                let start = word * 8;
                let chunk: [u8; 8] = data.get(start..start + 8)?.try_into().ok()?;
                let value = u64::from_be_bytes(chunk);
                let id = ((value >> offset) & mask) as usize;
                out.push(if id < palette.len() {
                    palette[id]
                } else {
                    i32::try_from(id).ok()?
                });
            }
            Some(out)
        }
    }
}

/// Block state id → block name and the state's own properties, read from
/// the same `blocks.json` report the id tables are built from.
pub struct StateTable {
    /// `state id → (block name, properties of that state)`.
    map: HashMap<i32, (String, Vec<(String, String)>)>,
}

impl StateTable {
    /// Loads `blocks.json`; every state id the reports know becomes a row.
    pub fn load(path: &Path) -> Self {
        let text =
            std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let report: serde_json::Value = serde_json::from_str(&text).expect("blocks report");
        let mut map = HashMap::new();
        for (name, block) in report.as_object().expect("blocks object") {
            let states = block.get("states").and_then(|s| s.as_array());
            for state in states.into_iter().flatten() {
                let id = state["id"].as_i64().unwrap_or(-1) as i32;
                let props = state["properties"]
                    .as_object()
                    .map(|object| {
                        object
                            .iter()
                            .map(|(key, value)| {
                                (key.clone(), value.as_str().unwrap_or_default().to_owned())
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                map.insert(id, (name.clone(), props));
            }
        }
        Self { map }
    }

    /// Name and properties of one state id.
    pub fn get(&self, id: i32) -> Option<(&str, &[(String, String)])> {
        self.map
            .get(&id)
            .map(|(name, props)| (name.as_str(), props.as_slice()))
    }

    /// Whether the state's only fluid property says `waterlogged=true`.
    pub fn waterlogged(&self, id: i32) -> bool {
        self.map.get(&id).is_some_and(|(_, props)| {
            props
                .iter()
                .any(|(key, value)| key == "waterlogged" && value == "true")
        })
    }

    /// The four names vanilla's non-air rule excludes
    /// (parity test `section_map`, recorded payloads).
    pub fn is_air(&self, id: i32) -> bool {
        matches!(
            self.get(id).map(|(name, _)| name),
            None | Some("minecraft:air") | Some("minecraft:cave_air") | Some("minecraft:void_air")
        )
    }

    /// The names beyond `water`/`lava` whose fluid state vanilla's fluid
    /// header counts: four aquatic plants and `bubble_column` (every delta
    /// of the evidence report is exactly these cells).
    pub fn is_aquatic(&self, id: i32) -> bool {
        matches!(
            self.get(id).map(|(name, _)| name),
            Some("minecraft:seagrass")
                | Some("minecraft:tall_seagrass")
                | Some("minecraft:kelp")
                | Some("minecraft:kelp_plant")
                | Some("minecraft:bubble_column")
        )
    }

    /// The fluid names the current rule counts.
    pub fn is_water_or_lava(&self, id: i32) -> bool {
        matches!(
            self.get(id).map(|(name, _)| name),
            Some("minecraft:water") | Some("minecraft:lava")
        )
    }
}

/// The three header numbers vanilla declares and the candidates computed
/// from the very same cells: `non-air` and two fluid rules.
pub fn header_evidence(section: &ChunkSection, table: &StateTable) -> HeaderEvidence {
    let Some(states) = cells(&section.blocks, 4096) else {
        return HeaderEvidence::default();
    };
    let (mut non_air, mut fluid_name, mut fluid_logged, mut fluid_full) =
        (0_u16, 0_u16, 0_u16, 0_u16);
    for id in states {
        if !table.is_air(id) {
            non_air += 1;
        }
        if table.is_water_or_lava(id) {
            fluid_name += 1;
        }
        if table.is_water_or_lava(id) || table.waterlogged(id) {
            fluid_logged += 1;
        }
        if table.is_water_or_lava(id) || table.waterlogged(id) || table.is_aquatic(id) {
            fluid_full += 1;
        }
    }
    HeaderEvidence {
        non_air,
        fluid_name,
        fluid_logged,
        fluid_full,
    }
}

/// Counts computed from one section's own cells under each rule.
#[derive(Debug, Default, Clone, Copy)]
pub struct HeaderEvidence {
    /// Non-air cells (name outside air/cave_air/void_air).
    pub non_air: u16,
    /// Cells whose name is `:water` or `:lava` — our current rule.
    pub fluid_name: u16,
    /// The same plus states with `waterlogged=true`.
    pub fluid_logged: u16,
    /// The same plus the aquatic names: `seagrass`, `tall_seagrass`,
    /// `kelp`, `kelp_plant`, `bubble_column`.
    pub fluid_full: u16,
}
