//! Reading of registries and tags from the client jar.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;

use protocol::NbtTag;
use serde_json::Value;
use zip::ZipArchive;

use crate::tags::{self, RawTag};
use crate::{DataError, GameData, RegistryData, RegistryEntry, NAMESPACE};

/// Directories under `data/minecraft/` of the jar whose registries are
/// synchronized to the client in the Registry Data packet. The list comes
/// from the public protocol documentation
/// (<https://minecraft.wiki/wiki/Java_Edition_protocol>); the entries
/// themselves are read from the user's own copy of the game.
pub const SYNCHRONIZED_REGISTRIES: &[&str] = &[
    "banner_pattern",
    "chat_type",
    "damage_type",
    "dialog",
    "dimension_type",
    "enchantment",
    "instrument",
    "jukebox_song",
    "painting_variant",
    "sulfur_cube_archetype",
    "test_environment",
    "test_instance",
    "timeline",
    "trim_material",
    "trim_pattern",
    "world_clock",
    "worldgen/biome",
    "cat_variant",
    "cat_sound_variant",
    "chicken_variant",
    "chicken_sound_variant",
    "cow_variant",
    "cow_sound_variant",
    "frog_variant",
    "pig_variant",
    "pig_sound_variant",
    "wolf_variant",
    "wolf_sound_variant",
    "zombie_nautilus_variant",
];

/// Where registry and tag JSON files live inside the jar.
pub(crate) const JAR_DATA_PREFIX: &str = "data/minecraft/";
/// Where tag JSON files live inside the jar.
pub(crate) const TAG_PREFIX: &str = "data/minecraft/tags/";

/// Loads registries and tags from the client jar at `path`.
pub fn load_from_jar(path: impl AsRef<Path>) -> Result<GameData, DataError> {
    let path = path.as_ref();
    tracing::info!(jar = %path.display(), "loading game data");
    let file = File::open(path).map_err(|source| DataError::Open {
        path: path.display().to_string(),
        source,
    })?;
    load_from_zip(file)
}

/// Loads registries and tags from an open zip stream — the content of the
/// client jar.
pub fn load_from_zip<R: Read + Seek>(reader: R) -> Result<GameData, DataError> {
    let mut archive = ZipArchive::new(reader)?;
    let mut names: Vec<String> = archive
        .file_names()
        .map(|name| name.map(|name| name.into_owned()))
        .collect::<Result<Vec<String>, _>>()?;
    names.sort();

    let mut registries = BTreeMap::new();
    for dir in SYNCHRONIZED_REGISTRIES {
        let entries = load_registry(&mut archive, &names, dir)?;
        tracing::debug!(registry = dir, entries = entries.len(), "registry loaded");
        registries.insert(format!("{NAMESPACE}:{dir}"), RegistryData { entries });
    }

    let raw_tags = collect_tags(&mut archive, &names)?;
    let tags = tags::resolve(raw_tags, &registries)?;

    let total: usize = registries
        .values()
        .map(|registry| registry.entries.len())
        .sum();
    tracing::info!(
        registries = registries.len(),
        entries = total,
        tag_registries = tags.len(),
        "game data loaded"
    );
    Ok(GameData { registries, tags })
}

/// Reads every flat JSON file under `data/minecraft/<dir>/` and converts it
/// into a registry entry sorted by identifier.
fn load_registry<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    names: &[String],
    dir: &str,
) -> Result<Vec<RegistryEntry>, DataError> {
    let prefix = format!("{JAR_DATA_PREFIX}{dir}/");
    let mut entries = Vec::new();
    for name in names {
        let Some(rest) = name.strip_prefix(prefix.as_str()) else {
            continue;
        };
        let Some(stem) = rest.strip_suffix(".json") else {
            continue;
        };
        if stem.contains('/') {
            return Err(DataError::NestedEntry { path: name.clone() });
        }
        let value = read_json(archive, name)?;
        let data = NbtTag::from_json(&value).map_err(|source| DataError::Nbt {
            path: name.clone(),
            source,
        })?;
        entries.push(RegistryEntry {
            name: format!("{NAMESPACE}:{stem}"),
            data,
        });
    }
    if entries.is_empty() {
        return Err(DataError::MissingRegistry {
            registry: format!("{NAMESPACE}:{dir}"),
            prefix,
        });
    }
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
}

/// Reads every tag file of a synchronized registry; tags of other
/// registries (block, item, ...) are left to the client, which already
/// knows them.
fn collect_tags<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    names: &[String],
) -> Result<BTreeMap<String, BTreeMap<String, RawTag>>, DataError> {
    let mut collected: BTreeMap<String, BTreeMap<String, RawTag>> = BTreeMap::new();
    for name in names {
        let Some(rest) = name.strip_prefix(TAG_PREFIX) else {
            continue;
        };
        let Some(stem) = rest.strip_suffix(".json") else {
            continue;
        };
        let Some((dir, tag)) = tags::split_tag_path(stem) else {
            continue;
        };
        let text = read_text(archive, name)?;
        let file: tags::TagFile =
            serde_json::from_str(&text).map_err(|source| DataError::Json {
                path: name.clone(),
                source,
            })?;
        let registry_id = format!("{NAMESPACE}:{dir}");
        collected
            .entry(registry_id)
            .or_default()
            .insert(format!("{NAMESPACE}:{tag}"), file.into_raw(name.clone()));
    }
    Ok(collected)
}

/// Reads a jar entry as text.
fn read_text<R: Read + Seek>(archive: &mut ZipArchive<R>, name: &str) -> Result<String, DataError> {
    let mut file = archive.by_name(name)?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|source| DataError::Read {
            path: name.to_owned(),
            source,
        })?;
    Ok(text)
}

/// Reads and parses a JSON file of the jar.
fn read_json<R: Read + Seek>(archive: &mut ZipArchive<R>, name: &str) -> Result<Value, DataError> {
    let text = read_text(archive, name)?;
    serde_json::from_str(&text).map_err(|source| DataError::Json {
        path: name.to_owned(),
        source,
    })
}
