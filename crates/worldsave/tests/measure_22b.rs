//! Measurement of the Шаг 22b costs, by the number of changed cells in one
//! chunk (ignored: run it on demand).
//!
//! `cargo test -p worldsave --release --test measure_22b -- --ignored --nocapture`
//!
//! Three numbers the owner asked for:
//!
//! * **snapshot** — [`ChunkStore::autosave_batch`], what the tick pays to hand
//!   one dirty chunk to the writer (holds the dirty lock for the copy);
//! * **write** — [`worldsave::Store::write_diff`] on the writer thread:
//!   encode, write to `.tmp`, `fsync`, rename (the price of the atomic write);
//! * **apply** — [`worldsave::Store::apply`] at startup for one chunk, i.e. the
//!   read + decode + palette rewrite of a saved diff.
//!
//! The chunk is the synthetic 24-section payload the other tests use; the
//! cells sit in the first section. The cost is per changed cell, so the shape
//! of the chunk does not matter for the numbers (the palette rewrite of a
//! section is the part that depends on it, and that is the same section
//! either way).

use std::sync::Arc;
use std::time::{Duration, Instant};

use data::BlockStates;
use net::chunks::ReadyChunks;
use net::store::ChunkStore;
use protocol::{ChunkSection, HeightmapRecord, LevelChunkWithLight, LightData, PaletteContainer};
use worldsave::{ChunkDiff, Store, WorldIdentity};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after the epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "minerust-worldsave-measure-{name}-{}-{stamp}",
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
            r#"
            {
              "minecraft:air": {"states": [{"id": 0, "default": true}]},
              "minecraft:stone": {"states": [{"id": 1, "default": true}]}
            }
            "#,
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

fn chunk(chunk_x: i32, chunk_z: i32) -> LevelChunkWithLight {
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

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

#[test]
#[ignore = "measurement: run with --release --ignored --nocapture"]
fn world_save_costs_by_changed_cells() {
    println!("| cells | snapshot us | write ms | apply us |");
    println!("|---|---|---|---|");
    for cells in [1_usize, 16, 64, 256] {
        // The tick side: one dirty chunk, the batch copied out of the lock.
        let store = ChunkStore::new(ReadyChunks::new(vec![chunk(0, 0)]), blocks());
        for cell in 0..cells {
            let x = (cell & 15) as i32;
            let z = ((cell >> 4) & 15) as i32;
            store.set_block(x, -64, z, 1).expect("dirty cell");
        }
        let mut snapshots = Vec::with_capacity(200);
        for _ in 0..200 {
            let started = Instant::now();
            let batch = store.autosave_batch(256);
            snapshots.push(started.elapsed());
            assert_eq!(batch.len(), 1, "the chunk stays dirty without an ack");
            std::hint::black_box(&batch);
            store.not_queued(&batch);
        }

        let changes = store.shutdown_batch().remove(0).changes;

        // The writer side: the same diff, atomically written again and again.
        let dir = temp_dir(&format!("cells{cells}"));
        let save = Store::open(&dir, &identity(), false).expect("store");
        let diff = ChunkDiff {
            chunk_x: 0,
            chunk_z: 0,
            changes: changes.clone(),
        };
        let mut writes = Vec::with_capacity(20);
        for _ in 0..20 {
            let started = Instant::now();
            save.write_diff(&diff).expect("written");
            writes.push(started.elapsed());
        }

        // The startup side: decode the file and lay it over a fresh template.
        let mut applies = Vec::with_capacity(20);
        for _ in 0..20 {
            let mut template = vec![Arc::new(chunk(0, 0))];
            let started = Instant::now();
            let saved = save.apply(&mut template, &blocks()).expect("apply");
            applies.push(started.elapsed());
            assert_eq!(saved.len(), 1, "the diff applies");
            std::hint::black_box(&saved);
        }
        let _ = std::fs::remove_dir_all(&dir);

        println!(
            "| {cells} | {:.1} | {:.2} | {:.1} |",
            median(snapshots).as_secs_f64() * 1e6,
            median(writes).as_secs_f64() * 1e3,
            median(applies).as_secs_f64() * 1e6,
        );
    }
}
