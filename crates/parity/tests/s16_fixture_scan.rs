//! Finds natural v3 cells for the live S16 persistence scenario.
//!
//! This ignored helper reads the configured template through the production
//! loader and prints the nearest coordinates of the requested block names. It
//! never writes the template or a save directory.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use net::store::{chunk_cell_state, MIN_SECTION_Y};
use parity::load_parity_env;
use protocol::BLOCK_CELLS;

const TARGETS: [&str; 5] = [
    "minecraft:stone",
    "minecraft:dirt",
    "minecraft:short_grass",
    "minecraft:seagrass",
    "minecraft:kelp",
];

/// One candidate cell ordered by `distance² from spawn, x, y, z, state id`.
type Candidate = (i64, i32, i32, i32, i32);

#[tokio::test]
#[ignore = "needs the owner's v3 template and generated reports"]
async fn nearest_s16_fixture_cells() {
    let env = load_parity_env().expect("parity environment");
    let config = env.config;
    let data = Arc::new(
        data::load_from_jar(
            &config.client_jar,
            config.registries_report.as_deref().map(Path::new),
        )
        .expect("game data"),
    );
    let (loader, world) = server::build_world(&config, data).expect("world builds");
    let loader = loader.expect("world loader");
    let reply = loader.load_batch().await.expect("warm-up queued");
    let store = reply.await.expect("warm-up reply").expect("world loads");
    let snapshot = store.snapshot();
    let spawn = world.spawn;
    let spawn_chunk = (spawn.x >> 4, spawn.z >> 4);
    let mut found: BTreeMap<&str, Vec<Candidate>> =
        TARGETS.into_iter().map(|name| (name, Vec::new())).collect();

    for chunk in snapshot.chunks() {
        if (chunk.chunk_x - spawn_chunk.0).abs() > 12 || (chunk.chunk_z - spawn_chunk.1).abs() > 12
        {
            continue;
        }
        for cell in 0..net::store::chunk_cell_count(chunk) {
            let Some(state) = chunk_cell_state(chunk, cell) else {
                continue;
            };
            let Some(name) = store.blocks().block_name(state) else {
                continue;
            };
            let Some(rows) = found.get_mut(name) else {
                continue;
            };
            let slot = cell as usize / BLOCK_CELLS;
            let local = cell as usize % BLOCK_CELLS;
            let x = chunk.chunk_x * 16 + (local & 15) as i32;
            let z = chunk.chunk_z * 16 + ((local >> 4) & 15) as i32;
            let y = (MIN_SECTION_Y + slot as i32) * 16 + ((local >> 8) & 15) as i32;
            let dx = i64::from(x - spawn.x);
            let dz = i64::from(z - spawn.z);
            let dy = i64::from(y - spawn.y);
            let score = dx * dx + dz * dz + dy * dy;
            rows.push((score, x, y, z, state));
        }
    }

    println!("spawn=({}, {}, {})", spawn.x, spawn.y, spawn.z);
    for (name, rows) in &mut found {
        rows.sort();
        rows.truncate(12);
        println!("{name}: {rows:?}");
        assert!(!rows.is_empty(), "no fixture cell for {name}");
    }
}
