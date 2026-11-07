//! Error type of the data loading crate.

use std::io;

use protocol::NbtError;
use zip::result::ZipError;

/// Errors that can occur while reading registries and tags from the game
/// jar. Every file-related variant names the offending jar path so a broken
/// or unexpected jar is easy to diagnose.
#[derive(Debug, thiserror::Error)]
pub enum DataError {
    /// The jar file itself cannot be opened (missing path, permissions).
    #[error("cannot open game jar {path}: {source}")]
    Open {
        /// Path that failed to open.
        path: String,
        /// Underlying I/O error.
        source: io::Error,
    },

    /// The jar is not a readable zip archive.
    #[error("cannot read the game jar: {0}")]
    Zip(#[from] ZipError),

    /// An entry of the jar cannot be read or decompressed.
    #[error("cannot read {path} from the game jar: {source}")]
    Read {
        /// Jar path of the entry.
        path: String,
        /// Underlying I/O error.
        source: io::Error,
    },

    /// A JSON file in the jar is not valid JSON or has an unexpected shape.
    #[error("cannot parse JSON in {path}: {source}")]
    Json {
        /// Jar path of the file.
        path: String,
        /// Underlying JSON error.
        source: serde_json::Error,
    },

    /// A registry entry JSON cannot be represented as NBT.
    #[error("cannot convert {path} to NBT: {source}")]
    Nbt {
        /// Jar path of the file.
        path: String,
        /// Underlying NBT conversion error.
        source: NbtError,
    },

    /// A synchronized registry has no entries in the jar.
    #[error("registry {registry} has no entries under {prefix} in the game jar")]
    MissingRegistry {
        /// Identifier of the registry, e.g. `minecraft:biome`.
        registry: String,
        /// Jar prefix that was searched.
        prefix: String,
    },

    /// Registry entries must be flat files, but a nested path was found.
    #[error("registry entries must be flat, but {path} is nested")]
    NestedEntry {
        /// Jar path of the nested file.
        path: String,
    },

    /// A tag refers to a registry entry that does not exist.
    #[error("entry {name} of registry {registry}, referenced by {path}, does not exist")]
    MissingEntry {
        /// Identifier of the registry, e.g. `minecraft:damage_type`.
        registry: String,
        /// Missing entry identifier.
        name: String,
        /// Jar path of the tag file that references it.
        path: String,
    },

    /// A tag refers to another tag that does not exist.
    #[error("tag #{tag} of registry {registry}, referenced by {path}, does not exist")]
    MissingTag {
        /// Identifier of the registry, e.g. `minecraft:damage_type`.
        registry: String,
        /// Missing tag identifier.
        tag: String,
        /// Jar path of the tag file that references it.
        path: String,
    },

    /// Tags of one registry refer to each other in a cycle.
    #[error("cycle between tags of registry {registry}: #{tag}")]
    TagCycle {
        /// Identifier of the registry, e.g. `minecraft:damage_type`.
        registry: String,
        /// Tag where the cycle was detected.
        tag: String,
    },
}
