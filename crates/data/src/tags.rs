//! Tag files: parsing and resolution to numeric entry ids.
//!
//! A tag file lists entries of one registry and may reference other tags of
//! the same registry with a leading `#`. Resolution expands those
//! references into the numeric ids the Update Tags packet sends.

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use crate::load::SYNCHRONIZED_REGISTRIES;
use crate::{DataError, RegistryData, NAMESPACE};

/// A tag file as it appears in the data pack: an optional `replace` flag
/// (accepted and ignored — the jar is the single source of tags) and the
/// list of values.
#[derive(Debug, Deserialize)]
pub(crate) struct TagFile {
    #[serde(default)]
    values: Vec<TagValue>,
}

impl TagFile {
    /// Wraps the parsed file with the jar path it came from.
    pub(crate) fn into_raw(self, path: String) -> RawTag {
        RawTag {
            path,
            values: self.values,
        }
    }
}

/// One value of a tag file.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum TagValue {
    /// A plain entry name or a reference to another tag of the same
    /// registry (`#minecraft:signs`). Names without a namespace default to
    /// `minecraft`.
    Name(String),
    /// An entry with an explicit `required` flag; a missing optional entry
    /// is dropped instead of failing the whole load.
    Entry {
        /// Entry name or `#`-prefixed tag reference.
        id: String,
        /// Whether the reference must resolve (the default).
        #[serde(default = "required_by_default")]
        required: bool,
    },
}

fn required_by_default() -> bool {
    true
}

/// A tag file before resolution: raw values plus the jar path they came
/// from (for error messages).
pub(crate) struct RawTag {
    /// Jar path of the tag file.
    path: String,
    /// Values in file order.
    values: Vec<TagValue>,
}

/// Splits a tag path (without the `data/minecraft/tags/` prefix and the
/// `.json` suffix) into the registry directory it belongs to and the tag
/// name, e.g. `banner_pattern/pattern_item/skull` becomes the directory
/// `banner_pattern` and the tag `pattern_item/skull`. Returns `None` for
/// tags of registries that are not synchronized (block, item, ...).
pub(crate) fn split_tag_path(rest: &str) -> Option<(&'static str, &str)> {
    let (first, remainder) = rest.split_once('/')?;
    if let Some((second, tag)) = remainder.split_once('/') {
        // Two-segment registry directories, e.g. `worldgen/biome`.
        let two = format!("{first}/{second}");
        if let Some(dir) = SYNCHRONIZED_REGISTRIES.iter().copied().find(|d| *d == two) {
            return Some((dir, tag));
        }
    }
    // One-segment registry directory, e.g. `damage_type`; `remainder` keeps
    // any further nesting as part of the tag name.
    let dir = SYNCHRONIZED_REGISTRIES
        .iter()
        .copied()
        .find(|d| *d == first)?;
    Some((dir, remainder))
}

/// Resolves raw tags into numeric entry ids: references are expanded in
/// place, duplicates removed, missing required references rejected, and
/// tags that end up empty are dropped.
pub(crate) fn resolve(
    raw_tags: BTreeMap<String, BTreeMap<String, RawTag>>,
    registries: &BTreeMap<String, RegistryData>,
) -> Result<BTreeMap<String, BTreeMap<String, Vec<i32>>>, DataError> {
    let mut resolved_all = BTreeMap::new();
    for (registry_id, registry_tags) in raw_tags {
        // Guaranteed by `collect_tags`, which only groups tags of the
        // synchronized registries that `load_registry` just loaded.
        let Some(registry) = registries.get(registry_id.as_str()) else {
            continue;
        };
        let mut memo: BTreeMap<String, Vec<i32>> = BTreeMap::new();
        let mut resolved = BTreeMap::new();
        for (tag_id, raw) in &registry_tags {
            let ids = resolve_tag(
                &registry_id,
                tag_id,
                &raw.path,
                &registry_tags,
                registry,
                &mut memo,
                &mut Vec::new(),
            )?;
            if !ids.is_empty() {
                resolved.insert(tag_id.clone(), ids);
            }
        }
        if !resolved.is_empty() {
            resolved_all.insert(registry_id, resolved);
        }
    }
    Ok(resolved_all)
}

/// Resolves one tag, memoizing results and detecting reference cycles.
/// `path` is the file that references the tag (the tag's own file when
/// called for a top level tag).
fn resolve_tag(
    registry_id: &str,
    tag_id: &str,
    path: &str,
    raw_tags: &BTreeMap<String, RawTag>,
    registry: &RegistryData,
    memo: &mut BTreeMap<String, Vec<i32>>,
    stack: &mut Vec<String>,
) -> Result<Vec<i32>, DataError> {
    if let Some(ids) = memo.get(tag_id) {
        return Ok(ids.clone());
    }
    if stack.iter().any(|open| open == tag_id) {
        return Err(DataError::TagCycle {
            registry: registry_id.to_owned(),
            tag: tag_id.to_owned(),
        });
    }
    let Some(raw) = raw_tags.get(tag_id) else {
        return Err(DataError::MissingTag {
            registry: registry_id.to_owned(),
            tag: tag_id.to_owned(),
            path: path.to_owned(),
        });
    };
    stack.push(tag_id.to_owned());
    let ids = collect_values(raw, registry_id, registry, raw_tags, memo, stack);
    stack.pop();
    let ids = ids?;
    memo.insert(tag_id.to_owned(), ids.clone());
    Ok(ids)
}

/// Resolves every value of one tag file in order.
fn collect_values(
    raw: &RawTag,
    registry_id: &str,
    registry: &RegistryData,
    raw_tags: &BTreeMap<String, RawTag>,
    memo: &mut BTreeMap<String, Vec<i32>>,
    stack: &mut Vec<String>,
) -> Result<Vec<i32>, DataError> {
    let mut ids = Vec::new();
    let mut seen = BTreeSet::new();
    for value in &raw.values {
        let (text, required) = match value {
            TagValue::Name(name) => (name.as_str(), true),
            TagValue::Entry { id, required } => (id.as_str(), *required),
        };
        match resolve_value(
            text,
            registry_id,
            &raw.path,
            registry,
            raw_tags,
            memo,
            stack,
        ) {
            Ok(resolved) => {
                for id in resolved {
                    if seen.insert(id) {
                        ids.push(id);
                    }
                }
            }
            Err(error)
                if !required
                    && matches!(
                        error,
                        DataError::MissingEntry { .. } | DataError::MissingTag { .. }
                    ) =>
            {
                // An optional reference to something that does not exist
                // is simply left out of the tag.
            }
            Err(error) => return Err(error),
        }
    }
    Ok(ids)
}

/// Resolves a single value: a `#`-prefixed reference becomes the ids of
/// that tag, anything else must be an entry of the registry.
fn resolve_value(
    text: &str,
    registry_id: &str,
    path: &str,
    registry: &RegistryData,
    raw_tags: &BTreeMap<String, RawTag>,
    memo: &mut BTreeMap<String, Vec<i32>>,
    stack: &mut Vec<String>,
) -> Result<Vec<i32>, DataError> {
    if let Some(reference) = text.strip_prefix('#') {
        let tag_id = qualify(reference);
        return resolve_tag(registry_id, &tag_id, path, raw_tags, registry, memo, stack);
    }
    let name = qualify(text);
    registry
        .entry_id(&name)
        .map(|id| vec![id])
        .ok_or_else(|| DataError::MissingEntry {
            registry: registry_id.to_owned(),
            name,
            path: path.to_owned(),
        })
}

/// Data pack names may omit the namespace; every entry and tag of the jar
/// lives in `minecraft`.
fn qualify(name: &str) -> String {
    if name.contains(':') {
        name.to_owned()
    } else {
        format!("{NAMESPACE}:{name}")
    }
}
