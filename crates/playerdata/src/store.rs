//! The on-disk store: `save_dir/players/data/<uuid>.dat`.
//!
//! The format is the vanilla one (gzip-compressed disk NBT, named compound
//! root). Reading keeps every unknown tag of an existing file; writing
//! updates only the fields this server models and writes the rest back
//! untouched. Writes are atomic: a temporary sibling is flushed, synced and
//! renamed over the target, and the directory entry is synced where the
//! platform supports it.
//!
//! A save root created by this server carries `minerust-save.json`, a marker
//! with the format version. A non-empty root without the marker is still
//! used (the owner may point `save_dir` at an existing world), but the server
//! logs it and does not claim the directory.
//!
//! SOURCE: the owner's world saves and the S7 join trace
//! (`docs/PLAYERDATA.md` §1): field names and shapes were read out of the
//! real `.dat` files, never copied from the game's code.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use protocol::{NbtTag, Uuid};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::error::PlayerdataError;
use crate::state::{
    Pose, SavedItem, SavedPlayer, Snapshot, NEW_PLAYER_FOOD, NEW_PLAYER_HEALTH,
    NEW_PLAYER_SATURATION, OVERWORLD,
};

/// Marker file of a save root created by this server.
pub const MARKER_FILE: &str = "minerust-save.json";

/// Format version of [`MARKER_FILE`]; bump it when the marker's shape or the
/// meaning of the directory changes.
///
/// Version 2 (Шаг 22b) adds no field: the same marker describes the player
/// files and the world diffs of one `save_dir`. A version 1 marker is
/// migrated in place, keeping its `data_version`.
pub const MARKER_VERSION: u32 = 2;

/// The version whose marker is rewritten as [`MARKER_VERSION`] on open.
const MIGRATED_VERSION: u32 = 1;

/// The `format` field of [`Marker`].
const MARKER_FORMAT: &str = "minerust-save";

/// Contents of [`MARKER_FILE`].
#[derive(Debug, Serialize, Deserialize)]
struct Marker {
    /// Always [`MARKER_FORMAT`]; a different value is not our directory.
    format: String,
    /// [`MARKER_VERSION`] at the time the directory was created.
    version: u32,
    /// `DataVersion` of the world the root was created for.
    data_version: i32,
}

/// Size of the standard player health/food envelope (vanilla 0..=20).
const MAX_FOOD: i32 = 20;
/// Maximum health and saturation the vanilla format uses.
const MAX_HEALTH: f32 = 20.0;

/// The player save directory of one world.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
    players: PathBuf,
    data_version: i32,
}

impl Store {
    /// Opens the save root, creating `players/data` when needed.
    ///
    /// `forbidden` are directories the save root must never be equal to or
    /// inside: the world template and the vanilla parity stand. The check is
    /// made before the root is created (on the resolved path) and again after
    /// canonicalisation (which catches a symlinked root). The server refuses
    /// to start on a collision — player files must never land in the
    /// template or in the stand's own world.
    pub fn open(
        root: impl AsRef<Path>,
        data_version: i32,
        forbidden: &[(&'static str, &Path)],
    ) -> Result<Self, PlayerdataError> {
        let root = root.as_ref();
        let resolved = resolve(root)?;
        check_forbidden(&resolved, forbidden)?;
        fs::create_dir_all(&resolved)
            .map_err(|source| PlayerdataError::io(root.display(), source))?;
        let root = fs::canonicalize(&resolved)
            .map_err(|source| PlayerdataError::io(resolved.display(), source))?;
        // A symlink can turn a clean path into one inside a forbidden
        // directory; the canonical form is the one that counts.
        check_forbidden(&root, forbidden)?;
        let root = fs::canonicalize(&root)
            .map_err(|source| PlayerdataError::io(root.display(), source))?;
        check_marker(&root, data_version)?;
        let players = root.join("players").join("data");
        fs::create_dir_all(&players)
            .map_err(|source| PlayerdataError::io(players.display(), source))?;
        Ok(Self {
            root,
            players,
            data_version,
        })
    }

    /// The save root (`save_dir`).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The `players/data` directory.
    pub fn players_dir(&self) -> &Path {
        &self.players
    }

    /// The data schema written into every file (`level.dat`'s
    /// `Data.DataVersion` of the world the server serves).
    pub fn data_version(&self) -> i32 {
        self.data_version
    }

    /// Path of one player's file.
    pub fn path(&self, uuid: Uuid) -> PathBuf {
        self.players.join(format!("{}.dat", file_name(uuid)))
    }

    /// Reads one player file; `None` means the player has no save yet.
    ///
    /// A damaged file is an error, not a guess: the caller logs it and uses
    /// the spawn. Every unknown tag stays in the file — this reader only
    /// looks at the fields it needs.
    pub fn read(&self, uuid: Uuid) -> Result<Option<SavedPlayer>, PlayerdataError> {
        let path = self.path(uuid);
        let Some(root) = self.read_document(&path)? else {
            return Ok(None);
        };
        let NbtTag::Compound(map) = root else {
            return Err(PlayerdataError::corrupt(
                path.display(),
                "root is not a compound",
            ));
        };
        self.parse_player(uuid, &path, &map).map(Some)
    }

    /// Writes one snapshot, merging it into an existing file.
    ///
    /// A file that cannot be read is logged and replaced by a fresh minimal
    /// compound (there is no way to merge unknown tags of a broken
    /// document). The temporary file is in the same directory, so the
    /// rename is atomic on one volume.
    pub fn write(&self, snapshot: &Snapshot) -> Result<(), PlayerdataError> {
        validate(snapshot)?;
        let path = self.path(snapshot.uuid);
        let mut map = match self.read_document(&path) {
            Ok(Some(NbtTag::Compound(map))) => map,
            Ok(Some(_)) => {
                warn!(
                    path = %path.display(),
                    "existing player file has a non-compound root, writing a fresh one"
                );
                BTreeMap::new()
            }
            Ok(None) => BTreeMap::new(),
            Err(err) => {
                warn!(
                    %err,
                    path = %path.display(),
                    "existing player file is unreadable, writing a fresh one"
                );
                BTreeMap::new()
            }
        };
        update(&mut map, snapshot, self.data_version);
        let document = NbtTag::Compound(map)
            .to_disk_bytes("")
            .map_err(|err| PlayerdataError::corrupt(path.display(), err.to_string()))?;
        write_atomically(&path, &document)
    }

    /// Reads and decodes the gzip disk NBT document under `path`.
    fn read_document(&self, path: &Path) -> Result<Option<NbtTag>, PlayerdataError> {
        let compressed = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(PlayerdataError::io(path.display(), source)),
        };
        let mut raw = Vec::new();
        GzDecoder::new(&compressed[..])
            .read_to_end(&mut raw)
            .map_err(|err| PlayerdataError::corrupt(path.display(), err.to_string()))?;
        let tag = NbtTag::decode_disk(&raw)
            .map_err(|err| PlayerdataError::corrupt(path.display(), err.to_string()))?;
        Ok(Some(tag))
    }

    /// Extracts the fields the join reads, warning about values it replaces.
    fn parse_player(
        &self,
        uuid: Uuid,
        path: &Path,
        map: &BTreeMap<String, NbtTag>,
    ) -> Result<SavedPlayer, PlayerdataError> {
        let pose = parse_pose(map)
            .ok_or_else(|| PlayerdataError::corrupt(path.display(), "no finite Pos/Rotation"))?;
        let health = match number(map.get("Health")) {
            Some(value) if value.is_finite() && value > 0.0 => {
                if value > MAX_HEALTH {
                    warn!(player = %hex_uuid(uuid), value, "Health above 20, clamped");
                    MAX_HEALTH
                } else {
                    value
                }
            }
            _ => {
                warn!(
                    player = %hex_uuid(uuid),
                    "Health is missing, not a number or not positive; using 20"
                );
                NEW_PLAYER_HEALTH
            }
        };
        let food = match map.get("foodLevel") {
            Some(NbtTag::Int(value)) if (0..=MAX_FOOD).contains(value) => *value,
            Some(NbtTag::Int(value)) => {
                warn!(player = %hex_uuid(uuid), value, "foodLevel out of 0..=20, clamped");
                (*value).clamp(0, MAX_FOOD)
            }
            _ => {
                warn!(player = %hex_uuid(uuid), "foodLevel is missing or not an int; using 20");
                NEW_PLAYER_FOOD
            }
        };
        let saturation = match number(map.get("foodSaturationLevel")) {
            Some(value) if value.is_finite() && value >= 0.0 => value.min(MAX_HEALTH),
            _ => {
                warn!(
                    player = %hex_uuid(uuid),
                    "foodSaturationLevel is missing or invalid; using 5"
                );
                NEW_PLAYER_SATURATION
            }
        };
        let on_ground = matches!(map.get("OnGround"), Some(NbtTag::Byte(value)) if *value != 0);
        let game_mode = match map.get("playerGameType") {
            Some(NbtTag::Int(value)) if (0..=3).contains(value) => Some(*value),
            _ => None,
        };
        let dimension = match map.get("Dimension") {
            Some(NbtTag::String(value)) => value.clone(),
            _ => {
                warn!(
                    player = %hex_uuid(uuid),
                    "Dimension is missing or not a string; treating the file as not overworld"
                );
                String::new()
            }
        };
        Ok(SavedPlayer {
            pose,
            health,
            food,
            saturation,
            on_ground,
            game_mode,
            dimension,
            inventory: parse_inventory(map, uuid),
        })
    }
}

/// The vanilla file name of a UUID: the UUID's wire bytes in order, with
/// dashes after the 4th, 6th and 8th byte. The first three groups are
/// therefore reversed relative to the UUID string the game prints:
/// `[10 92 05 08 d5 d8 3e ed ...]` (the wire bytes of
/// `08059210-d8d5-ed3e-...`) is the file
/// `10920508-d5d8-3eed-....dat`.
///
/// SOURCE: the vanilla server's own files in
/// `E:\MineRust-data\vanilla\world\players\data` (Alice `10920508-...`,
/// Bob `faa5dca3-...`) and the frozen template's ParityBot file
/// (`docs/PLAYERDATA.md` §1.1).
pub fn file_name(uuid: Uuid) -> String {
    let mut name = String::with_capacity(36);
    for (index, byte) in uuid.iter().enumerate() {
        if index == 4 || index == 6 || index == 8 || index == 10 {
            name.push('-');
        }
        push_hex(byte, &mut name);
    }
    name
}

/// Lower-case hexadecimal, the form the client/vanilla uses in file names.
fn push_hex(byte: &u8, out: &mut String) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    out.push(DIGITS[usize::from(byte >> 4)] as char);
    out.push(DIGITS[usize::from(byte & 0x0F)] as char);
}

/// Log-friendly UUID text (the plain group order).
fn hex_uuid(uuid: Uuid) -> String {
    let reversed = [&uuid[0..4], &uuid[4..6], &uuid[6..8]];
    let mut out = String::with_capacity(36);
    for (index, group) in reversed.iter().enumerate() {
        if index > 0 {
            out.push('-');
        }
        for byte in group.iter() {
            push_hex(byte, &mut out);
        }
    }
    for group in [&uuid[8..10], &uuid[10..16]] {
        out.push('-');
        for byte in group.iter() {
            push_hex(byte, &mut out);
        }
    }
    out
}

/// Applies the snapshot to the compound, adding missing keys and keeping
/// every unknown tag.
fn update(map: &mut BTreeMap<String, NbtTag>, snapshot: &Snapshot, data_version: i32) {
    let pose = snapshot.pose;
    map.insert(
        "Pos".to_owned(),
        NbtTag::List(vec![
            NbtTag::Double(pose.x),
            NbtTag::Double(pose.y),
            NbtTag::Double(pose.z),
        ]),
    );
    map.insert(
        "Rotation".to_owned(),
        NbtTag::List(vec![NbtTag::Float(pose.yaw), NbtTag::Float(pose.pitch)]),
    );
    map.insert("Health".to_owned(), NbtTag::Float(snapshot.health));
    map.insert("foodLevel".to_owned(), NbtTag::Int(snapshot.food));
    map.insert(
        "foodSaturationLevel".to_owned(),
        NbtTag::Float(snapshot.saturation),
    );
    map.insert("playerGameType".to_owned(), NbtTag::Int(snapshot.game_mode));
    map.insert("Dimension".to_owned(), NbtTag::String(OVERWORLD.to_owned()));
    map.insert(
        "Inventory".to_owned(),
        inventory_tags(map.get("Inventory"), &snapshot.inventory),
    );
    map.insert(
        "OnGround".to_owned(),
        NbtTag::Byte(if snapshot.on_ground { 1 } else { 0 }),
    );
    map.insert("DataVersion".to_owned(), NbtTag::Int(data_version));
    map.insert(
        "UUID".to_owned(),
        NbtTag::IntArray(uuid_ints(snapshot.uuid)),
    );
    // The few fields a vanilla file always carries and a fresh compound
    // would otherwise lack; they are only defaults, an existing value is
    // never overwritten.
    map.entry("Air".to_owned()).or_insert(NbtTag::Short(300));
    map.entry("Motion".to_owned()).or_insert(NbtTag::List(vec![
        NbtTag::Double(0.0),
        NbtTag::Double(0.0),
        NbtTag::Double(0.0),
    ]));
    map.entry("fall_distance".to_owned())
        .or_insert(NbtTag::Double(0.0));
    map.entry("XpLevel".to_owned()).or_insert(NbtTag::Int(0));
    map.entry("XpTotal".to_owned()).or_insert(NbtTag::Int(0));
    map.entry("XpP".to_owned()).or_insert(NbtTag::Float(0.0));
}

/// The `Inventory` list written back: the snapshot's hotbar entries plus
/// every existing entry the server does not model (all slots above the
/// hotbar), in slot order.
///
/// Шаг 24d: only the hotbar is saved, as the plan says. Keeping the other
/// entries byte-for-byte means a file from the vanilla server does not lose
/// the armor or the main inventory when this server saves it.
fn inventory_tags(existing: Option<&NbtTag>, hotbar: &[SavedItem]) -> NbtTag {
    let mut entries: Vec<(i32, NbtTag)> = Vec::new();
    if let Some(NbtTag::List(list)) = existing {
        for entry in list {
            let slot = match entry {
                NbtTag::Compound(fields) => match fields.get("Slot") {
                    Some(NbtTag::Byte(value)) => Some(i32::from(*value)),
                    Some(NbtTag::Int(value)) => Some(*value),
                    _ => None,
                },
                _ => None,
            };
            if let Some(slot) = slot {
                if !(0..=8).contains(&slot) {
                    entries.push((slot, entry.clone()));
                }
            }
        }
    }
    for item in hotbar {
        if !(0..=8).contains(&item.slot) {
            continue;
        }
        entries.push((
            item.slot,
            NbtTag::Compound(BTreeMap::from([
                ("count".to_owned(), NbtTag::Int(item.count)),
                ("Slot".to_owned(), NbtTag::Byte(item.slot as i8)),
                ("id".to_owned(), NbtTag::String(item.id.clone())),
            ])),
        ));
    }
    entries.sort_by_key(|(slot, _)| *slot);
    NbtTag::List(entries.into_iter().map(|(_, tag)| tag).collect())
}

/// The four big-endian ints of the UUID, the layout the vanilla file uses.
fn uuid_ints(uuid: Uuid) -> Vec<i32> {
    uuid.as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| i32::from_be_bytes(*chunk))
        .collect()
}

/// Validates the snapshot before anything touches the disk.
fn validate(snapshot: &Snapshot) -> Result<(), PlayerdataError> {
    let pose = snapshot.pose;
    if ![pose.x, pose.y, pose.z]
        .iter()
        .all(|value| value.is_finite())
    {
        return Err(PlayerdataError::InvalidValue {
            field: "Pos",
            detail: "a coordinate is not finite".to_owned(),
        });
    }
    if ![pose.yaw, pose.pitch].iter().all(|value| value.is_finite()) {
        return Err(PlayerdataError::InvalidValue {
            field: "Rotation",
            detail: "an angle is not finite".to_owned(),
        });
    }
    if !snapshot.health.is_finite() || !(0.0..=MAX_HEALTH).contains(&snapshot.health) {
        return Err(PlayerdataError::InvalidValue {
            field: "Health",
            detail: format!("{} is outside 0..=20", snapshot.health),
        });
    }
    if !(0..=MAX_FOOD).contains(&snapshot.food) {
        return Err(PlayerdataError::InvalidValue {
            field: "foodLevel",
            detail: format!("{} is outside 0..=20", snapshot.food),
        });
    }
    if !snapshot.saturation.is_finite() || snapshot.saturation < 0.0 {
        return Err(PlayerdataError::InvalidValue {
            field: "foodSaturationLevel",
            detail: format!("{} is not a non-negative number", snapshot.saturation),
        });
    }
    if !(0..=3).contains(&snapshot.game_mode) {
        return Err(PlayerdataError::InvalidValue {
            field: "playerGameType",
            detail: format!("{} is outside 0..=3", snapshot.game_mode),
        });
    }
    for item in &snapshot.inventory {
        if !(0..=8).contains(&item.slot) {
            return Err(PlayerdataError::InvalidValue {
                field: "Inventory",
                detail: format!("Slot {} is outside the saved hotbar 0..=8", item.slot),
            });
        }
        if !(1..=99).contains(&item.count) {
            return Err(PlayerdataError::InvalidValue {
                field: "Inventory",
                detail: format!("count {} is outside 1..=99", item.count),
            });
        }
        if item.id.is_empty() {
            return Err(PlayerdataError::InvalidValue {
                field: "Inventory",
                detail: "an entry has an empty item id".to_owned(),
            });
        }
    }
    Ok(())
}

/// The `Inventory` list of the file, as far as the server models it (Шаг 24d).
///
/// SOURCE: a real 26.3 player file (`docs/PLAYERDATA.md` §…; read during
/// Шаг 24.0d) carries `Inventory` as a list of compounds
/// `{count: int, Slot: byte, id: string}`. Entries the server cannot read are
/// skipped with a warning instead of failing the whole file, so an unknown or
/// newer entry never locks a player out.
fn parse_inventory(map: &BTreeMap<String, NbtTag>, uuid: Uuid) -> Vec<SavedItem> {
    let Some(NbtTag::List(entries)) = map.get("Inventory") else {
        return Vec::new();
    };
    let mut items = Vec::with_capacity(entries.len());
    for entry in entries {
        let NbtTag::Compound(fields) = entry else {
            warn!(player = %hex_uuid(uuid), "Inventory entry is not a compound, skipped");
            continue;
        };
        let slot = match fields.get("Slot") {
            Some(NbtTag::Byte(value)) => i32::from(*value),
            Some(NbtTag::Int(value)) => *value,
            _ => {
                warn!(player = %hex_uuid(uuid), "Inventory entry has no readable Slot, skipped");
                continue;
            }
        };
        let count = match fields.get("count") {
            Some(NbtTag::Int(value)) => *value,
            Some(NbtTag::Byte(value)) => i32::from(*value),
            _ => {
                warn!(player = %hex_uuid(uuid), slot, "Inventory entry has no readable count, skipped");
                continue;
            }
        };
        let id = match fields.get("id") {
            Some(NbtTag::String(value)) => value.clone(),
            _ => {
                warn!(player = %hex_uuid(uuid), slot, "Inventory entry has no item id, skipped");
                continue;
            }
        };
        if count <= 0 {
            continue;
        }
        // Only the hotbar is modelled (Шаг 24d); the other entries of the
        // file are kept verbatim by `inventory_tags` on the next write.
        if !(0..=8).contains(&slot) {
            continue;
        }
        items.push(SavedItem { slot, count, id });
    }
    items
}

/// `Pos` and `Rotation` of the file, if both are complete and finite.
fn parse_pose(map: &BTreeMap<String, NbtTag>) -> Option<Pose> {
    let pos = list_doubles(map.get("Pos"))?;
    let rotation = list_floats(map.get("Rotation"))?;
    if pos.len() != 3 || rotation.len() != 2 {
        return None;
    }
    if !pos.iter().all(|value| value.is_finite()) || !rotation.iter().all(|value| value.is_finite())
    {
        return None;
    }
    Some(Pose {
        x: pos[0],
        y: pos[1],
        z: pos[2],
        yaw: rotation[0],
        pitch: rotation[1],
    })
}

/// A homogeneous list of doubles.
fn list_doubles(tag: Option<&NbtTag>) -> Option<Vec<f64>> {
    match tag? {
        NbtTag::List(items) => items
            .iter()
            .map(|item| match item {
                NbtTag::Double(value) => Some(*value),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// A homogeneous list of floats (the vanilla `Rotation`).
fn list_floats(tag: Option<&NbtTag>) -> Option<Vec<f32>> {
    match tag? {
        NbtTag::List(items) => items
            .iter()
            .map(|item| match item {
                NbtTag::Float(value) => Some(*value),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// A float/double tag as `f32` (the saved `Health` is a float; a double is
/// accepted rather than guessed at).
fn number(tag: Option<&NbtTag>) -> Option<f32> {
    match tag? {
        NbtTag::Float(value) => Some(*value),
        NbtTag::Double(value) => Some(*value as f32),
        _ => None,
    }
}

/// Rejects a save root that is `forbidden` or inside it.
fn check_forbidden(
    root: &Path,
    forbidden: &[(&'static str, &Path)],
) -> Result<(), PlayerdataError> {
    for (label, path) in forbidden {
        let path = resolve(path)?;
        if root == path {
            return Err(PlayerdataError::ForbiddenPath {
                save_dir: root.display().to_string(),
                relation: "equal to",
                label,
                forbidden: path.display().to_string(),
            });
        }
        if root.starts_with(&path) {
            return Err(PlayerdataError::ForbiddenPath {
                save_dir: root.display().to_string(),
                relation: "inside",
                label,
                forbidden: path.display().to_string(),
            });
        }
    }
    Ok(())
}

/// Canonicalises a path even when its leaf does not exist yet.
///
/// The deepest existing ancestor is canonicalised and the remaining
/// components are appended. Both sides of a comparison therefore share the
/// canonical form: `%TEMP%` may spell a directory differently (a short name)
/// than `canonicalize` does, and a raw prefix match would then miss.
fn resolve(path: &Path) -> Result<PathBuf, PlayerdataError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| PlayerdataError::io(path.display(), source))?
            .join(path)
    };
    let mut suffix: Vec<OsString> = Vec::new();
    let mut cursor = absolute.as_path();
    loop {
        if let Ok(base) = fs::canonicalize(cursor) {
            let mut result = base;
            for component in suffix.iter().rev() {
                if component.as_os_str() == OsStr::new(".") {
                    continue;
                }
                if component.as_os_str() == OsStr::new("..") {
                    result.pop();
                    continue;
                }
                result.push(component);
            }
            return Ok(result);
        }
        let Some(name) = cursor.file_name() else {
            break;
        };
        suffix.push(name.to_os_string());
        let Some(parent) = cursor.parent() else {
            break;
        };
        cursor = parent;
    }
    // Nothing along the path exists: fold the components by hand.
    let mut folded = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                folded.pop();
            }
            other => folded.push(other.as_os_str()),
        }
    }
    Ok(folded)
}

/// Checks or creates [`MARKER_FILE`] in the save root.
///
/// An existing marker is only read: an unknown format/version or a damaged
/// document is logged and the store keeps working (the save files are still
/// the vanilla ones). An empty directory is claimed by writing the marker; a
/// non-empty directory without one is logged and left untouched.
fn check_marker(root: &Path, data_version: i32) -> Result<(), PlayerdataError> {
    let marker = root.join(MARKER_FILE);
    match fs::read(&marker) {
        Ok(bytes) => match serde_json::from_slice::<Marker>(&bytes) {
            Ok(found) if found.format == MARKER_FORMAT && found.version == MARKER_VERSION => Ok(()),
            Ok(found) if found.format == MARKER_FORMAT && found.version == MIGRATED_VERSION => {
                // Version 1 and 2 share every field; the rewrite records the
                // new version and keeps the world's `data_version` (Шаг 22b).
                let document = serde_json::to_vec_pretty(&Marker {
                    format: MARKER_FORMAT.to_owned(),
                    version: MARKER_VERSION,
                    data_version: found.data_version,
                })
                .map_err(|err| PlayerdataError::corrupt(marker.display(), err.to_string()))?;
                write_plain_atomically(&marker, &document)?;
                info!(
                    path = %marker.display(),
                    from = MIGRATED_VERSION,
                    to = MARKER_VERSION,
                    data_version = found.data_version,
                    "save marker migrated"
                );
                Ok(())
            }
            Ok(found) => {
                warn!(
                    path = %marker.display(),
                    format = %found.format,
                    version = found.version,
                    expected = MARKER_VERSION,
                    "save marker has an unknown format or version; continuing"
                );
                Ok(())
            }
            Err(err) => {
                warn!(
                    %err,
                    path = %marker.display(),
                    "save marker is unreadable; continuing"
                );
                Ok(())
            }
        },
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            let empty = fs::read_dir(root)
                .map_err(|source| PlayerdataError::io(root.display(), source))?
                .next()
                .is_none();
            if !empty {
                warn!(
                    path = %root.display(),
                    marker = MARKER_FILE,
                    "save_dir is not empty and has no marker; using it anyway"
                );
                return Ok(());
            }
            let document = serde_json::to_vec_pretty(&Marker {
                format: MARKER_FORMAT.to_owned(),
                version: MARKER_VERSION,
                data_version,
            })
            .map_err(|err| PlayerdataError::corrupt(marker.display(), err.to_string()))?;
            write_plain_atomically(&marker, &document)
        }
        Err(source) => Err(PlayerdataError::io(marker.display(), source)),
    }
}

/// Writes `bytes` through a synced temporary sibling and renames it over
/// `path` (no compression; used for the marker).
fn write_plain_atomically(path: &Path, bytes: &[u8]) -> Result<(), PlayerdataError> {
    let temporary = path.with_extension("json.tmp");
    let mut file = fs::File::create(&temporary)
        .map_err(|source| PlayerdataError::io(temporary.display(), source))?;
    file.write_all(bytes)
        .map_err(|source| PlayerdataError::io(temporary.display(), source))?;
    file.sync_all()
        .map_err(|source| PlayerdataError::io(temporary.display(), source))?;
    drop(file);
    fs::rename(&temporary, path).map_err(|source| {
        let _ = fs::remove_file(&temporary);
        PlayerdataError::io(path.display(), source)
    })?;
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

/// Flushes the directory entry of a renamed file.
///
/// Unix needs an explicit `fsync` on the directory for the new name to
/// survive a power loss; Windows journals the rename itself (and opening a
/// directory for `fsync` is not portable there), so the call is a no-op.
fn sync_dir(path: &Path) -> Result<(), PlayerdataError> {
    #[cfg(unix)]
    {
        let dir =
            fs::File::open(path).map_err(|source| PlayerdataError::io(path.display(), source))?;
        dir.sync_all()
            .map_err(|source| PlayerdataError::io(path.display(), source))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Writes `document` through a synced temporary sibling and renames it over
/// `path`.
fn write_atomically(path: &Path, document: &[u8]) -> Result<(), PlayerdataError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|source| PlayerdataError::io(parent.display(), source))?;
    }
    let temporary = path.with_extension("dat.tmp");
    let file = fs::File::create(&temporary)
        .map_err(|source| PlayerdataError::io(temporary.display(), source))?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder
        .write_all(document)
        .map_err(|source| PlayerdataError::io(temporary.display(), source))?;
    let file = encoder
        .finish()
        .map_err(|source| PlayerdataError::io(temporary.display(), source))?;
    file.sync_all()
        .map_err(|source| PlayerdataError::io(temporary.display(), source))?;
    drop(file);
    fs::rename(&temporary, path).map_err(|source| {
        let _ = fs::remove_file(&temporary);
        PlayerdataError::io(path.display(), source)
    })?;
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique directory under the system temp root.
    fn temp_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "minerust-playerdata-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn uuid() -> Uuid {
        [
            0x10, 0x92, 0x05, 0x08, 0xd5, 0xd8, 0x3e, 0xed, 0x4d, 0x95, 0xbc, 0x0c, 0x5b, 0x0c,
            0x1f, 0x6f,
        ]
    }

    fn snapshot(uuid: Uuid) -> Snapshot {
        Snapshot {
            uuid,
            pose: Pose {
                x: 96.5,
                y: 128.0,
                z: -10.3,
                yaw: 45.0,
                pitch: -12.5,
            },
            on_ground: true,
            health: 15.666_667,
            food: 17,
            saturation: 0.0,
            game_mode: 0,
            inventory: Vec::new(),
        }
    }

    #[test]
    fn the_file_name_matches_the_vanilla_player_file() {
        assert_eq!(file_name(uuid()), "10920508-d5d8-3eed-4d95-bc0c5b0c1f6f");

        // The frozen template's ParityBot file, byte for byte:
        // `players/data/ff1070c9-7b1a-3e30-a01e-89fe7a426681.dat`
        // (docs/PLAYERDATA.md §1.1). `uuid()` above is the same shape for
        // Alice's bytes, so this is the vanilla naming rule, not a round trip.
        let parity_bot = [
            0xff, 0x10, 0x70, 0xc9, 0x7b, 0x1a, 0x3e, 0x30, 0xa0, 0x1e, 0x89, 0xfe, 0x7a, 0x42,
            0x66, 0x81,
        ];
        assert_eq!(
            file_name(parity_bot),
            "ff1070c9-7b1a-3e30-a01e-89fe7a426681"
        );
    }

    #[test]
    fn a_snapshot_round_trips_through_the_gzip_document() {
        let dir = temp_dir("roundtrip");
        let store = Store::open(&dir, 5023, &[]).expect("store");
        let snap = snapshot(uuid());
        store.write(&snap).expect("write");

        let path = store.path(uuid());
        assert!(path.is_file(), "the file exists");
        let compressed = fs::read(&path).expect("read");
        assert_eq!(&compressed[..2], &[0x1f, 0x8b], "a gzip container");
        assert!(
            !path.with_extension("dat.tmp").exists(),
            "the temporary file is gone after the rename"
        );

        let saved = store.read(uuid()).expect("read").expect("a file");
        assert_eq!(saved.pose, snap.pose);
        assert_eq!(saved.health, snap.health);
        assert_eq!(saved.food, snap.food);
        assert_eq!(saved.saturation, snap.saturation);
        assert_eq!(saved.on_ground, snap.on_ground);
        assert_eq!(saved.game_mode, Some(snap.game_mode));
        assert!(saved.is_overworld());
    }

    #[test]
    fn a_missing_file_is_none_and_an_empty_store_exists() {
        let dir = temp_dir("missing");
        let store = Store::open(&dir, 5023, &[]).expect("store");
        assert!(store.players_dir().is_dir());
        assert_eq!(store.read(uuid()).expect("read"), None);
    }

    #[test]
    fn writing_keeps_the_unknown_tags_of_an_existing_file() {
        let dir = temp_dir("merge");
        let store = Store::open(&dir, 5023, &[]).expect("store");
        let path = store.path(uuid());
        let mut inventory = BTreeMap::new();
        // Slot 40 is the offhand in the player-file slot space: a slot the
        // server does not model (Шаг 24d), so it must survive the merge.
        inventory.insert("Slot".to_owned(), NbtTag::Byte(40));
        inventory.insert("count".to_owned(), NbtTag::Int(1));
        inventory.insert(
            "id".to_owned(),
            NbtTag::String("minecraft:shield".to_owned()),
        );
        let mut abilities = BTreeMap::new();
        abilities.insert("mayfly".to_owned(), NbtTag::Byte(1));
        let mut map = BTreeMap::new();
        map.insert(
            "Pos".to_owned(),
            NbtTag::List(vec![
                NbtTag::Double(1.0),
                NbtTag::Double(2.0),
                NbtTag::Double(3.0),
            ]),
        );
        map.insert(
            "Rotation".to_owned(),
            NbtTag::List(vec![NbtTag::Float(0.0), NbtTag::Float(0.0)]),
        );
        map.insert(
            "Inventory".to_owned(),
            NbtTag::List(vec![NbtTag::Compound(inventory)]),
        );
        map.insert("abilities".to_owned(), NbtTag::Compound(abilities));
        map.insert("DataVersion".to_owned(), NbtTag::Int(1));
        fs::write(
            &path,
            gzip(&NbtTag::Compound(map).to_disk_bytes("").expect("disk")),
        )
        .expect("seed file");

        store.write(&snapshot(uuid())).expect("write");

        let raw = fs::read(&path).expect("read");
        let mut decoded = Vec::new();
        GzDecoder::new(&raw[..])
            .read_to_end(&mut decoded)
            .expect("gunzip");
        let NbtTag::Compound(map) = NbtTag::decode_disk(&decoded).expect("nbt") else {
            panic!("compound root");
        };
        assert!(map.contains_key("Inventory"), "unknown list kept");
        assert!(map.contains_key("abilities"), "unknown compound kept");
        assert_eq!(map["DataVersion"], NbtTag::Int(5023));
        let NbtTag::List(inventory) = &map["Inventory"] else {
            panic!("Inventory list");
        };
        assert_eq!(inventory.len(), 1, "the offhand entry is kept");
        assert_eq!(
            inventory[0],
            NbtTag::Compound({
                let mut entry = BTreeMap::new();
                entry.insert("Slot".to_owned(), NbtTag::Byte(40));
                entry.insert("count".to_owned(), NbtTag::Int(1));
                entry.insert(
                    "id".to_owned(),
                    NbtTag::String("minecraft:shield".to_owned()),
                );
                entry
            })
        );
        let NbtTag::List(pos) = &map["Pos"] else {
            panic!("Pos list");
        };
        assert_eq!(pos[0], NbtTag::Double(96.5), "known fields updated");
    }

    #[test]
    fn the_hotbar_round_trips_and_other_inventory_entries_are_kept() {
        // Шаг 24d: the hotbar is saved in the player-file slot space
        // (`0..=8`), read back at join, and every other entry of an existing
        // `Inventory` list stays untouched.
        let dir = temp_dir("inventory");
        let store = Store::open(&dir, 5023, &[]).expect("store");
        let path = store.path(uuid());
        let mut offhand = BTreeMap::new();
        offhand.insert("Slot".to_owned(), NbtTag::Byte(40));
        offhand.insert("count".to_owned(), NbtTag::Int(1));
        offhand.insert(
            "id".to_owned(),
            NbtTag::String("minecraft:shield".to_owned()),
        );
        let mut map = snapshot_fixture();
        map.insert(
            "Inventory".to_owned(),
            NbtTag::List(vec![NbtTag::Compound(offhand)]),
        );
        fs::write(
            &path,
            gzip(&NbtTag::Compound(map).to_disk_bytes("").expect("disk")),
        )
        .expect("seed file");

        let mut snap = snapshot(uuid());
        snap.inventory = vec![
            SavedItem {
                slot: 0,
                count: 64,
                id: "minecraft:stone".to_owned(),
            },
            SavedItem {
                slot: 4,
                count: 1,
                id: "minecraft:oak_stairs".to_owned(),
            },
        ];
        store.write(&snap).expect("write");

        let saved = store.read(uuid()).expect("read").expect("file");
        assert_eq!(saved.inventory, snap.inventory, "the hotbar round-trips");

        // The offhand entry of the seeded file is still there.
        let raw = fs::read(&path).expect("read");
        let mut decoded = Vec::new();
        GzDecoder::new(&raw[..])
            .read_to_end(&mut decoded)
            .expect("gunzip");
        let NbtTag::Compound(map) = NbtTag::decode_disk(&decoded).expect("nbt") else {
            panic!("compound root");
        };
        let NbtTag::List(entries) = &map["Inventory"] else {
            panic!("Inventory list");
        };
        let slots: Vec<i32> = entries
            .iter()
            .filter_map(|entry| match entry {
                NbtTag::Compound(fields) => match fields.get("Slot") {
                    Some(NbtTag::Byte(value)) => Some(i32::from(*value)),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(slots, vec![0, 4, 40]);
    }

    /// The compound fields `parse_player` reads, for tests that seed a file.
    fn snapshot_fixture() -> BTreeMap<String, NbtTag> {
        let mut map = BTreeMap::new();
        map.insert(
            "Pos".to_owned(),
            NbtTag::List(vec![
                NbtTag::Double(1.0),
                NbtTag::Double(2.0),
                NbtTag::Double(3.0),
            ]),
        );
        map.insert(
            "Rotation".to_owned(),
            NbtTag::List(vec![NbtTag::Float(0.0), NbtTag::Float(0.0)]),
        );
        map.insert("DataVersion".to_owned(), NbtTag::Int(1));
        map
    }

    #[test]
    fn a_corrupt_file_reads_as_an_error_and_is_replaced_on_write() {
        let dir = temp_dir("corrupt");
        let store = Store::open(&dir, 5023, &[]).expect("store");
        let path = store.path(uuid());
        fs::write(&path, b"not gzip").expect("garbage");
        assert!(matches!(
            store.read(uuid()),
            Err(PlayerdataError::Corrupt { .. })
        ));

        store.write(&snapshot(uuid())).expect("write");
        assert_eq!(store.read(uuid()).expect("read").expect("file").food, 17);
    }

    #[test]
    fn finite_values_are_checked_before_the_disk_is_touched() {
        let dir = temp_dir("validate");
        let store = Store::open(&dir, 5023, &[]).expect("store");
        let mut snap = snapshot(uuid());
        snap.pose.x = f64::NAN;
        assert!(matches!(
            store.write(&snap),
            Err(PlayerdataError::InvalidValue { field: "Pos", .. })
        ));
        assert!(!store.path(uuid()).exists(), "nothing was written");

        let mut snap = snapshot(uuid());
        snap.health = 21.0;
        assert!(matches!(
            store.write(&snap),
            Err(PlayerdataError::InvalidValue {
                field: "Health",
                ..
            })
        ));

        let mut snap = snapshot(uuid());
        snap.game_mode = 4;
        assert!(matches!(
            store.write(&snap),
            Err(PlayerdataError::InvalidValue {
                field: "playerGameType",
                ..
            })
        ));
    }

    #[test]
    fn a_zero_or_broken_health_reads_as_twenty() {
        let dir = temp_dir("health");
        let store = Store::open(&dir, 5023, &[]).expect("store");
        let mut snap = snapshot(uuid());
        snap.health = 1.0;
        store.write(&snap).expect("write");
        let path = store.path(uuid());

        let mut map = read_map(&path);
        map.insert("Health".to_owned(), NbtTag::Float(0.0));
        fs::write(
            &path,
            gzip(&NbtTag::Compound(map).to_disk_bytes("").expect("disk")),
        )
        .expect("write zero health");
        assert_eq!(
            store.read(uuid()).expect("read").expect("file").health,
            20.0
        );

        let mut map = read_map(&path);
        map.insert("Health".to_owned(), NbtTag::String("oops".to_owned()));
        fs::write(
            &path,
            gzip(&NbtTag::Compound(map).to_disk_bytes("").expect("disk")),
        )
        .expect("write broken health");
        assert_eq!(
            store.read(uuid()).expect("read").expect("file").health,
            20.0
        );
    }

    #[test]
    fn a_non_overworld_dimension_is_reported() {
        let dir = temp_dir("dimension");
        let store = Store::open(&dir, 5023, &[]).expect("store");
        store.write(&snapshot(uuid())).expect("write");
        let path = store.path(uuid());
        let mut map = read_map(&path);
        map.insert(
            "Dimension".to_owned(),
            NbtTag::String("minecraft:the_nether".to_owned()),
        );
        fs::write(
            &path,
            gzip(&NbtTag::Compound(map).to_disk_bytes("").expect("disk")),
        )
        .expect("write dimension");
        let saved = store.read(uuid()).expect("read").expect("file");
        assert!(!saved.is_overworld());
    }

    #[test]
    fn the_save_root_is_refused_inside_a_forbidden_directory() {
        let parent = temp_dir("paths");
        let template = parent.join("template");
        fs::create_dir_all(&template).expect("template");
        let forbidden = [("world template", template.as_path())];

        let error = Store::open(template.join("inside"), 5023, &forbidden)
            .expect_err("inside the template");
        assert!(matches!(
            error,
            PlayerdataError::ForbiddenPath {
                relation: "inside",
                ..
            }
        ));
        assert!(!template.join("inside").exists(), "nothing was created");

        let error = Store::open(&template, 5023, &forbidden).expect_err("equal");
        assert!(matches!(
            error,
            PlayerdataError::ForbiddenPath {
                relation: "equal to",
                ..
            }
        ));

        let sibling = parent.join("sibling");
        Store::open(&sibling, 5023, &forbidden).expect("a sibling is allowed");
    }

    #[test]
    fn open_writes_the_marker_into_an_empty_save_root() {
        let dir = temp_dir("marker");
        let store = Store::open(&dir, 5023, &[]).expect("store");
        let marker = store.root().join(MARKER_FILE);
        assert!(marker.is_file(), "the marker is written on creation");
        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(&marker).expect("read")).expect("marker JSON");
        assert_eq!(json["format"], MARKER_FORMAT);
        assert_eq!(json["version"], MARKER_VERSION);
        assert_eq!(json["data_version"], 5023);
        assert!(
            !marker.with_extension("json.tmp").exists(),
            "the marker's temporary file is gone after the rename"
        );

        // Re-opening a directory that is already ours is not a warning.
        Store::open(&dir, 5023, &[]).expect("re-open");
    }

    #[test]
    fn open_tolerates_a_non_empty_root_without_a_marker() {
        let dir = temp_dir("foreign");
        let seed = dir.join("level.dat");
        fs::write(&seed, b"not ours").expect("seed");

        let store = Store::open(&dir, 5023, &[]).expect("a foreign root is usable");

        assert!(store.players_dir().is_dir());
        assert!(
            !dir.join(MARKER_FILE).exists(),
            "a directory the server did not create is not claimed"
        );
        assert_eq!(fs::read(&seed).expect("kept"), b"not ours");
    }

    #[test]
    fn open_tolerates_a_damaged_or_future_marker() {
        let dir = temp_dir("bad-marker");
        fs::write(dir.join(MARKER_FILE), b"{").expect("seed");
        Store::open(&dir, 5023, &[]).expect("a damaged marker is logged, not fatal");

        fs::write(
            dir.join(MARKER_FILE),
            br#"{"format":"minerust-save","version":999,"data_version":1}"#,
        )
        .expect("seed future");
        Store::open(&dir, 5023, &[]).expect("a future marker version is logged, not fatal");
    }

    #[test]
    fn open_migrates_a_version_one_marker_without_losing_its_fields() {
        let dir = temp_dir("migrate");
        fs::write(
            dir.join(MARKER_FILE),
            br#"{"format":"minerust-save","version":1,"data_version":5023}"#,
        )
        .expect("seed version 1");

        Store::open(&dir, 5023, &[]).expect("a version 1 marker is migrated");

        let json: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.join(MARKER_FILE)).expect("read")).expect("json");
        assert_eq!(json["version"], MARKER_VERSION, "rewritten as version 2");
        assert_eq!(json["format"], MARKER_FORMAT);
        assert_eq!(json["data_version"], 5023, "the world's version is kept");
    }

    #[test]
    fn a_write_syncs_its_directory_where_supported() {
        let dir = temp_dir("dir-sync");
        let store = Store::open(&dir, 5023, &[]).expect("store");
        store.write(&snapshot(uuid())).expect("write");
        let path = store.path(uuid());
        assert!(path.is_file(), "the rename landed");
        assert!(
            !path.with_extension("dat.tmp").exists(),
            "the temporary file is gone"
        );
        assert!(
            store.read(uuid()).expect("read").is_some(),
            "the player file reads back"
        );
    }

    fn read_map(path: &Path) -> BTreeMap<String, NbtTag> {
        let raw = fs::read(path).expect("read");
        let mut decoded = Vec::new();
        GzDecoder::new(&raw[..])
            .read_to_end(&mut decoded)
            .expect("gunzip");
        let NbtTag::Compound(map) = NbtTag::decode_disk(&decoded).expect("nbt") else {
            panic!("compound root");
        };
        map
    }

    fn gzip(document: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(document).expect("gzip");
        encoder.finish().expect("finish")
    }
}
