use super::*;
use protocol::{ChunkSection, HeightmapRecord, LightData, PaletteContainer, BLOCK_CELLS};

fn temp_dir(name: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "minerust-worldsave-store-{name}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn identity() -> WorldIdentity {
    WorldIdentity {
        template: "E:/worlds/template".to_owned(),
        level_dat_sha256: "abc".to_owned(),
        data_version: 5023,
    }
}

fn blocks() -> BlockStates {
    BlockStates::parse(
        r#"{
          "minecraft:air": {"states": [{"id": 0, "default": true}]},
          "minecraft:stone": {"states": [{"id": 1, "default": true}]}
        }"#,
        "test".to_owned(),
    )
    .expect("report")
}

fn empty_section() -> ChunkSection {
    ChunkSection {
        non_air: 0,
        fluid: 0,
        blocks: PaletteContainer::Single(0),
        biomes: PaletteContainer::Single(1),
    }
}

fn test_chunk(chunk_x: i32, chunk_z: i32) -> Arc<LevelChunkWithLight> {
    Arc::new(LevelChunkWithLight {
        chunk_x,
        chunk_z,
        heightmaps: vec![HeightmapRecord {
            id: 4,
            words: vec![0],
        }],
        sections: (0..24).map(|_| empty_section()).collect(),
        block_entities: Vec::new(),
        light: LightData {
            sky_mask: Vec::new(),
            block_mask: Vec::new(),
            empty_sky_mask: Vec::new(),
            empty_block_mask: Vec::new(),
            sky_arrays: Vec::new(),
            block_arrays: Vec::new(),
        },
    })
}

fn flat_cell(x: i32, y: i32, z: i32) -> u32 {
    let slot = (y >> 4) + 4;
    let local = ((y & 15) << 8) | ((z & 15) << 4) | (x & 15);
    slot as u32 * BLOCK_CELLS as u32 + local as u32
}

/// A diff is written atomically, read back, and applied to the template
/// chunk without touching any other chunk.
#[test]
fn a_diff_roundtrips_through_the_save_root() {
    let dir = temp_dir("roundtrip");
    let store = Store::open(&dir, &identity(), false).expect("store");
    let diff = ChunkDiff {
        chunk_x: 0,
        chunk_z: 0,
        changes: vec![(flat_cell(1, 64, 2), 1)],
    };
    store.write_diff(&diff).expect("written");
    assert_eq!(
        store.read_diff(0, 0).expect("read").expect("file"),
        diff,
        "the written document decodes to itself"
    );

    let mut chunks = vec![test_chunk(0, 0), test_chunk(1, 0)];
    let saved = store.apply(&mut chunks, &blocks()).expect("apply");
    assert_eq!(saved.len(), 1, "one chunk carries a diff");
    assert_eq!((saved[0].chunk_x, saved[0].chunk_z), (0, 0));
    assert_eq!(saved[0].cells, diff.changes);
    assert_eq!(
        net::store::chunk_cell_state(&chunks[0], flat_cell(1, 64, 2)),
        Some(1),
        "the saved cell is applied"
    );
    assert_eq!(
        chunks[1].sections[0].non_air, 0,
        "a chunk without a file stays the template"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// An empty diff removes the file, and a missing file is not an error.
#[test]
fn an_empty_diff_removes_the_file() {
    let dir = temp_dir("empty");
    let store = Store::open(&dir, &identity(), false).expect("store");
    let diff = ChunkDiff {
        chunk_x: 2,
        chunk_z: -3,
        changes: vec![(flat_cell(0, -64, 0), 1)],
    };
    store.write_diff(&diff).expect("written");
    assert!(store.chunk_path(2, -3).is_file());
    store
        .write_diff(&ChunkDiff {
            chunk_x: 2,
            chunk_z: -3,
            changes: Vec::new(),
        })
        .expect("removed");
    assert!(!store.chunk_path(2, -3).exists());
    store
        .write_diff(&ChunkDiff {
            chunk_x: 2,
            chunk_z: -3,
            changes: Vec::new(),
        })
        .expect("already absent");
    assert!(store.read_diff(2, -3).expect("read").is_none());
    let _ = fs::remove_dir_all(&dir);
}

/// A damaged file, an unknown state id and a chunk outside the template
/// are each skipped, and the rest of the save still applies.
#[test]
fn a_bad_file_is_skipped_without_losing_the_others() {
    let dir = temp_dir("damaged");
    let store = Store::open(&dir, &identity(), false).expect("store");
    let good = ChunkDiff {
        chunk_x: 0,
        chunk_z: 0,
        changes: vec![(flat_cell(1, 64, 2), 1)],
    };
    store.write_diff(&good).expect("good");
    fs::write(store.chunk_path(1, 0), b"not a diff").expect("damaged");
    store
        .write_diff(&ChunkDiff {
            chunk_x: 2,
            chunk_z: 0,
            changes: vec![(flat_cell(0, 64, 0), 99)],
        })
        .expect("unknown state");
    store
        .write_diff(&ChunkDiff {
            chunk_x: 9,
            chunk_z: 9,
            changes: vec![(flat_cell(0, 64, 0), 1)],
        })
        .expect("outside");

    let mut chunks = vec![test_chunk(0, 0), test_chunk(1, 0)];
    let saved = store.apply(&mut chunks, &blocks()).expect("apply");
    assert_eq!(saved.len(), 1, "only the good diff applies");
    assert_eq!((saved[0].chunk_x, saved[0].chunk_z), (0, 0));
    assert!(store.chunk_path(1, 0).is_file(), "damaged file is kept");
    assert!(store.chunk_path(2, 0).is_file(), "unknown id file is kept");
    let _ = fs::remove_dir_all(&dir);
}

/// Losing access to the whole chunks directory is a startup error, not a
/// warning followed by serving the untouched template as if the save were
/// empty.
#[test]
fn an_unreadable_chunks_path_refuses_apply() {
    let dir = temp_dir("missing-directory");
    let store = Store::open(&dir, &identity(), false).expect("store");
    fs::remove_dir_all(dir.join(CHUNKS_DIR)).expect("remove chunks dir");
    fs::write(dir.join(CHUNKS_DIR), b"not a directory").expect("block path");
    let mut chunks = vec![test_chunk(0, 0)];
    assert!(matches!(
        store.apply(&mut chunks, &blocks()),
        Err(StoreError::Io { action: "list", .. })
    ));
    let _ = fs::remove_dir_all(&dir);
}

/// Another template refuses the start without the override, and the
/// override applies the saved diffs anyway.
#[test]
fn a_mismatched_save_needs_the_override() {
    let dir = temp_dir("mismatch");
    let store = Store::open(&dir, &identity(), false).expect("store");
    store
        .write_diff(&ChunkDiff {
            chunk_x: 0,
            chunk_z: 0,
            changes: vec![(flat_cell(1, 64, 2), 1)],
        })
        .expect("written");
    let other = WorldIdentity {
        level_dat_sha256: "def".to_owned(),
        ..identity()
    };
    assert!(matches!(
        Store::open(&dir, &other, false),
        Err(StoreError::Meta(MetaError::Mismatch { .. }))
    ));
    let opened = Store::open(&dir, &other, true).expect("override");
    assert!(opened.mismatch());
    let mut chunks = vec![test_chunk(0, 0)];
    assert_eq!(
        opened.apply(&mut chunks, &blocks()).expect("apply").len(),
        1
    );
    assert_eq!(
        net::store::chunk_cell_state(&chunks[0], flat_cell(1, 64, 2)),
        Some(1),
        "the override applies the diffs"
    );
    let _ = fs::remove_dir_all(&dir);
}
