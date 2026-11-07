//! Loading of game data from the user's copy of the game.
//!
//! To introduce a client to the world the server has to send the
//! synchronized registries (biomes, damage types, instruments, ...) and the
//! tags of all registries the client must bind (`block`, `item`, `dialog`,
//! ...): the built-in registries take their numeric ids from the data
//! generator's `reports/registries.json`. None of this data lives in the
//! repository: it is read from `data/minecraft/` inside the client jar the
//! server owner points at (the `client_jar` setting of the server
//! configuration) and from the report (the `registries_report` setting).
//!
//! Identifiers and numeric ids follow the public protocol documentation
//! (<https://minecraft.wiki/wiki/Java_Edition_protocol>): the Registry Data
//! packet assigns an entry's numeric id by its position in the packet, so
//! entries are sorted by identifier here to keep ids deterministic. The
//! Update Tags packet then refers to those ids.

mod error;
mod load;
mod packets;
mod registry_ids;
mod tags;

#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;

#[cfg(test)]
mod tests;

pub use error::DataError;
pub use load::{load_from_jar, load_from_zip, SYNCHRONIZED_REGISTRIES};
pub use packets::{load_packets_report, packets_from_json, PacketIds};
pub use registry_ids::{load_registries_report, registries_from_json, RegistryIds};

use std::collections::BTreeMap;

use protocol::NbtTag;

/// Namespace of everything the jar defines under `data/minecraft/`.
pub(crate) const NAMESPACE: &str = "minecraft";

/// A single registry entry: the identifier the client sees plus the entry
/// definition converted from data pack JSON to NBT.
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryEntry {
    /// Full identifier of the entry, e.g. `minecraft:plains`.
    pub name: String,
    /// Entry definition as network NBT (see [`NbtTag`]).
    pub data: NbtTag,
}

/// One synchronized registry: entries sorted by identifier, so an entry's
/// numeric id is simply its index — matching the Registry Data packet,
/// where ids are assigned by position.
#[derive(Debug, Default)]
pub struct RegistryData {
    pub(crate) entries: Vec<RegistryEntry>,
}

impl RegistryData {
    /// Entries sorted by identifier; the numeric id of an entry is its
    /// index in this slice.
    pub fn entries(&self) -> &[RegistryEntry] {
        &self.entries
    }

    /// Numeric id of `name` inside this registry, if the entry exists.
    pub fn entry_id(&self, name: &str) -> Option<i32> {
        let index = self
            .entries
            .partition_point(|entry| entry.name.as_str() < name);
        match self.entries.get(index) {
            Some(entry) if entry.name == name => Some(index as i32),
            _ => None,
        }
    }
}

/// Everything the server needs to introduce a client to the game: the
/// synchronized registries and the tags of those registries.
#[derive(Debug, Default)]
pub struct GameData {
    pub(crate) registries: BTreeMap<String, RegistryData>,
    pub(crate) tags: BTreeMap<String, BTreeMap<String, Vec<i32>>>,
}

impl GameData {
    /// Synchronized registries by identifier (`minecraft:banner_pattern`
    /// and friends) in deterministic identifier order.
    pub fn registries(&self) -> impl Iterator<Item = (&str, &RegistryData)> {
        self.registries.iter().map(|(id, data)| (id.as_str(), data))
    }

    /// A single registry by identifier, if it was loaded.
    pub fn registry(&self, id: &str) -> Option<&RegistryData> {
        self.registries.get(id)
    }

    /// Tags grouped by registry identifier: registry id → tag id → numeric
    /// ids of the tag's entries. A tag that resolves to no entries is kept
    /// with an empty list: the client binds it as an empty tag.
    pub fn tags(&self) -> &BTreeMap<String, BTreeMap<String, Vec<i32>>> {
        &self.tags
    }
}
