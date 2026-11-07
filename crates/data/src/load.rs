//! Reading of registries and tags from the client jar.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;

use protocol::NbtTag;
use serde_json::Value;
use zip::ZipArchive;

use crate::registry_ids::{load_registries_report, RegistryIds};
use crate::tags::{self, RawTag};
use crate::{DataError, GameData, RegistryData, RegistryEntry, NAMESPACE};

/// Directories under `data/minecraft/` of the jar whose registries are
/// synchronized to the client in the Registry Data packet: exactly the 32
/// registries a real 26.3 server sends (verified against the vanilla trace
/// by the `registry_data_parity` test); the entries themselves are read
/// from the user's own copy of the game.
pub const SYNCHRONIZED_REGISTRIES: &[&str] = &[
    "banner_pattern",
    "block_transformer",
    "chat_type",
    "damage_type",
    "decorated_pot_pattern",
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
    "worldgen/block_state_provider",
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
///
/// `registries_report` points at the data generator's
/// `reports/registries.json` (see [`RegistryIds`]); it adds the tags of the
/// built-in registries (`block`, `item`, ...) with their static protocol
/// ids. Without it only the synchronized registries get tags.
pub fn load_from_jar(
    path: impl AsRef<Path>,
    registries_report: Option<&Path>,
) -> Result<GameData, DataError> {
    let path = path.as_ref();
    tracing::info!(jar = %path.display(), "loading game data");
    let report = match registries_report {
        Some(report_path) => {
            let ids = load_registries_report(report_path)?;
            tracing::info!(
                path = %report_path.display(),
                registries = ids.len(),
                "registries report loaded"
            );
            Some(ids)
        }
        None => {
            tracing::warn!(
                "no registries_report configured: tags of the built-in registries \
                 (block, item, ...) will not be sent"
            );
            None
        }
    };
    let file = File::open(path).map_err(|source| DataError::Open {
        path: path.display().to_string(),
        source,
    })?;
    load_from_zip(file, report.as_ref())
}

/// Loads registries and tags from an open zip stream — the content of the
/// client jar. `report` adds the id lookup of the built-in registries the
/// report describes (a fixture test passes `None`).
pub fn load_from_zip<R: Read + Seek>(
    reader: R,
    report: Option<&RegistryIds>,
) -> Result<GameData, DataError> {
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

    let no_report = RegistryIds::default();
    let report = report.unwrap_or(&no_report);

    // A jar tag registry is sent if the Registry Data packet synchronizes
    // it or if the report provides the static ids its tags refer to. That
    // leaves out registry directories the client owns alone
    // (`villager_trade`, `worldgen/structure`, ...).
    let mut tag_dirs: BTreeSet<String> = SYNCHRONIZED_REGISTRIES
        .iter()
        .map(|dir| (*dir).to_owned())
        .collect();
    for id in report.registry_ids() {
        let Some(dir) = id
            .strip_prefix(NAMESPACE)
            .and_then(|rest| rest.strip_prefix(':'))
        else {
            continue;
        };
        // Only directories of the jar's `data/minecraft/tags/` tree.
        if !dir.is_empty() {
            tag_dirs.insert(dir.to_owned());
        }
    }

    let raw_tags = collect_tags(&mut archive, &names, &tag_dirs)?;

    // Numeric ids of every tagged registry: the report's static ids first,
    // then the positions of the Registry Data packet — a synchronized
    // registry is redefined by what the server sends, report or not.
    let mut entry_ids: BTreeMap<String, BTreeMap<String, i32>> = BTreeMap::new();
    for registry_id in raw_tags.keys() {
        if let Some(ids) = report.get(registry_id) {
            entry_ids.insert(registry_id.clone(), ids.clone());
        }
    }
    for dir in SYNCHRONIZED_REGISTRIES {
        let Some(registry) = registries.get(&format!("{NAMESPACE}:{dir}")) else {
            // `registries` is filled from this very list above.
            continue;
        };
        let ids = registry
            .entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.name.clone(), index as i32))
            .collect();
        entry_ids.insert(format!("{NAMESPACE}:{dir}"), ids);
    }
    let tags = tags::resolve(raw_tags, &entry_ids)?;

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

/// Reads every tag file whose registry is in `tag_dirs`; tags of other
/// registries are left to the client, which already owns them.
fn collect_tags<R: Read + Seek>(
    archive: &mut ZipArchive<R>,
    names: &[String],
    tag_dirs: &BTreeSet<String>,
) -> Result<BTreeMap<String, BTreeMap<String, RawTag>>, DataError> {
    let mut collected: BTreeMap<String, BTreeMap<String, RawTag>> = BTreeMap::new();
    for name in names {
        let Some(rest) = name.strip_prefix(TAG_PREFIX) else {
            continue;
        };
        let Some(stem) = rest.strip_suffix(".json") else {
            continue;
        };
        let Some((dir, tag)) = tags::split_tag_path(stem, tag_dirs) else {
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
