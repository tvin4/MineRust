//! Шаг 13a, замеры на замороженном мире v3: сколько стоит вход второго
//! клиента (попадание в кэш) и пересборка одного чанка после изменения блока.
//!
//! Оба теста игнорируются по умолчанию: они требуют установки владельца
//! (`C:\MineRust-data`). Запускать в release — читатель мира упирается в CPU:
//!
//! ```text
//! cargo test -p parity --release --test store_rebuild -- --ignored --nocapture
//! ```

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use parity::load_parity_env;

/// Замороженный мир v3 (65×65 полных чанков), только читается.
const V3_TEMPLATE: &str = r"C:\MineRust-data\world-template-v3";

/// Сколько входов второго клиента замеряем.
const JOINS: usize = 100;

/// Сколько пересборок чанка замеряем.
const REBUILDS: usize = 200;

/// Медиана выборки «микросекунд» (уже в микросекундах).
fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).expect("finite samples"));
    let middle = samples.len() / 2;
    samples[middle]
}

/// Шаг 13a: стоимость входа второго клиента (кэш уже прогрет) и пересборки
/// одного чанка на v3.
#[tokio::test]
#[ignore = "requires the owner's installation (C:\\MineRust-data)"]
async fn v3_second_join_and_one_chunk_rebuild() {
    if !Path::new(V3_TEMPLATE).is_dir() {
        eprintln!("SKIP: v3 template not found at {V3_TEMPLATE}");
        return;
    }
    let env = match load_parity_env() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            return;
        }
    };
    let mut config = env.config.clone();
    config.world_dir = Some(V3_TEMPLATE.to_owned());
    let data = Arc::new(
        data::load_from_jar(
            &config.client_jar,
            config.registries_report.as_deref().map(Path::new),
        )
        .expect("game data from the jar"),
    );

    let (loader, _world) =
        server::build_world(&config, data).unwrap_or_else(|e| panic!("v3 world builds: {e}"));
    let loader = loader.expect("a configured world_dir yields a loader");

    // Warm-up: fills the cache, exactly as `server::run` does before binding.
    let warm_started = Instant::now();
    let reply = loader.load_batch().await.expect("warm-up can be queued");
    let store = reply
        .await
        .expect("the warm-up reply is alive")
        .expect("the v3 world can be read");
    let warm = warm_started.elapsed();

    // Second-client join, store side: the cached read plus one snapshot.
    let mut join_samples = Vec::with_capacity(JOINS);
    for _ in 0..JOINS {
        let started = Instant::now();
        let reply = loader.load_batch().await.expect("a join can be queued");
        let joined = reply
            .await
            .expect("the join reply is alive")
            .expect("the v3 world can be read");
        let _snapshot = joined.snapshot();
        join_samples.push(started.elapsed().as_secs_f64() * 1e6);
    }
    let join_p50 = median(join_samples);

    // One-chunk rebuild: write two states into one cell of the first chunk.
    let snapshot = store.snapshot();
    let chunk = snapshot
        .chunks()
        .first()
        .expect("the v3 world holds chunks");
    let (x, z) = (chunk.chunk_x * 16 + 1, chunk.chunk_z * 16 + 1);
    let y = 64;
    let before = store.block(x, y, z).expect("the cell is inside the world");

    // First write: a `Single` section turns into a packed one.
    let first_started = Instant::now();
    store
        .set_block(x, y, z, before.wrapping_add(1))
        .expect("the first write lands");
    let first = first_started.elapsed();

    // Steady state: unpack, change, repack the whole section.
    let other = before;
    let mut rebuild_samples = Vec::with_capacity(REBUILDS);
    for _ in 0..REBUILDS {
        let started = Instant::now();
        store.set_block(x, y, z, other).expect("the write lands");
        rebuild_samples.push(started.elapsed().as_secs_f64() * 1e6);
        store
            .set_block(x, y, z, before.wrapping_add(1))
            .expect("the write lands");
    }
    let rebuild_p50 = median(rebuild_samples);

    eprintln!(
        "v3 ({}): warm-up {:.1} ms; second join p50 {:.1} µs over {JOINS}; \
         one-chunk rebuild: first {:.1} µs, steady p50 {:.1} µs over {REBUILDS} \
         ({} chunks)",
        if cfg!(debug_assertions) {
            "debug, unoptimized"
        } else {
            "release"
        },
        warm.as_secs_f64() * 1000.0,
        join_p50,
        first.as_secs_f64() * 1e6,
        rebuild_p50,
        store.len(),
    );
}
