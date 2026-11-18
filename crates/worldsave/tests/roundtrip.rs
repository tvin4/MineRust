//! End to end: a template batch, one block change, the writer thread, a new
//! start (Шаг 22b).
//!
//! This is the S16 scenario without a client: the world template is read from
//! memory here (the region files themselves are the `world` crate's business),
//! the changed cell goes through the same `ChunkStore` and writer the server
//! uses, and a fresh start applies the saved file to a fresh copy of the
//! template.

use std::sync::Arc;

use data::{BlockStates, CollisionShapes};
use net::chunks::ReadyChunks;
use net::game::WorldSave as _;
use net::store::{chunk_cell_state, set_chunk_cell, ChunkStore};
use protocol::{ChunkSection, HeightmapRecord, LevelChunkWithLight, LightData, PaletteContainer};
use worldsave::{WorldIdentity, DEFAULT_CAPACITY};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "minerust-worldsave-roundtrip-{name}-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn identity() -> WorldIdentity {
    WorldIdentity {
        template: "E:/worlds/template".to_owned(),
        level_dat_sha256: "abc".to_owned(),
        data_version: 5023,
    }
}

fn blocks() -> Arc<BlockStates> {
    Arc::new(
        BlockStates::parse(
            r#"{
              "minecraft:air": {"states": [{"id": 0, "default": true}]},
              "minecraft:stone": {"states": [{"id": 1, "default": true}]}
            }"#,
            "test".to_owned(),
        )
        .expect("report"),
    )
}

fn empty_section() -> ChunkSection {
    ChunkSection {
        non_air: 0,
        fluid: 0,
        blocks: PaletteContainer::Single(0),
        biomes: PaletteContainer::Single(1),
    }
}

fn template_chunk(chunk_x: i32, chunk_z: i32) -> LevelChunkWithLight {
    LevelChunkWithLight {
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
    }
}

fn template_chunk_with_stone() -> LevelChunkWithLight {
    let mut chunk = template_chunk(0, 0);
    set_chunk_cell(&mut chunk, flat_cell(8, 64, 8), 1, &blocks()).expect("template stone");
    chunk
}

fn flat_cell(x: i32, y: i32, z: i32) -> u32 {
    let slot = (y >> 4) + 4;
    let local = ((y & 15) << 8) | ((z & 15) << 4) | (x & 15);
    slot as u32 * 4096 + local as u32
}

/// A break survives the writer and a new store sees it; restoring the
/// template's stone deletes the now-empty diff file.
#[test]
fn a_break_survives_a_restart_and_restoring_the_template_deletes_the_file() {
    let save_dir = temp_dir("save");
    let store = worldsave::Store::open(&save_dir, &identity(), false).expect("save root");

    // The first run: one chunk of the template, one broken block.
    let first = ChunkStore::new(
        ReadyChunks::new(vec![template_chunk_with_stone(), template_chunk(1, 0)]),
        blocks(),
    );
    first.set_block(8, 64, 8, 0).expect("break to air");
    let batch = first.autosave_batch(256);
    assert_eq!(batch.len(), 1, "only the touched chunk is dirty");
    let (runtime, writer) = worldsave::spawn(store.clone(), DEFAULT_CAPACITY).expect("spawn");
    assert!(runtime.submit(batch));
    writer
        .shutdown(std::time::Duration::from_secs(10))
        .expect("writer stops");
    assert_eq!(
        store.read_diff(0, 0).expect("read").expect("file").changes,
        vec![(flat_cell(8, 64, 8), 0)]
    );

    // A new start: the template is read again and the file is laid over it.
    let reopened = worldsave::Store::open(&save_dir, &identity(), false).expect("reopen");
    let mut template = vec![
        Arc::new(template_chunk_with_stone()),
        Arc::new(template_chunk(1, 0)),
    ];
    let saved = reopened.apply(&mut template, &blocks()).expect("apply");
    assert_eq!(saved.len(), 1);
    let second = ChunkStore::with_saved(
        ReadyChunks::from_arcs(template),
        blocks(),
        Arc::new(CollisionShapes::default()),
        saved,
    );
    assert_eq!(
        second.block(8, 64, 8),
        Some(0),
        "the broken block is still gone on a new start"
    );
    assert_eq!(second.block(9, 64, 8), Some(0), "the rest is the template");
    assert_eq!(
        chunk_cell_state(&second.snapshot().chunks()[0], flat_cell(8, 64, 8)),
        Some(0)
    );

    // The template block comes back: the diff becomes empty and the file is
    // removed.
    second.set_block(8, 64, 8, 1).expect("stone again");
    let batch = second.autosave_batch(256);
    assert!(
        batch[0].changes.is_empty(),
        "no change against the template"
    );
    let (runtime, writer) = worldsave::spawn(reopened.clone(), DEFAULT_CAPACITY).expect("spawn");
    assert!(runtime.submit(batch));
    writer
        .shutdown(std::time::Duration::from_secs(10))
        .expect("writer stops");
    assert!(
        reopened.read_diff(0, 0).expect("read").is_none(),
        "a chunk back at the template has no file"
    );

    let _ = std::fs::remove_dir_all(&save_dir);
}
