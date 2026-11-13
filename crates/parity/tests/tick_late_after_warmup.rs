//! M4.5 step 7.4: does the tick thread wake late **without any player**?
//!
//! The "tick woke late" warnings seen in live runs appeared 7–9 s after the
//! server started with `players = 0` and `late_ms` up to ~103 ms. This runner
//! reproduces the same start-up order as `server::run` (world build → warm-up
//! read → port bound → tick thread started), then does **nothing** for
//! `IDLE` seconds and samples the tick metrics once per second:
//!
//! * the world warm-up duration (the 13 s read of the v3 template);
//! * every second: tick count, `late_ticks` delta, `late_max_ms`, jitter p99;
//! * the uptime at which the first late tick appeared.
//!
//! No bot connects, so any late tick is start-up noise, not player load. The
//! report lands in `C:\MineRust-data\traces\tick-idle-after-warmup.md`.
//!
//! ```text
//! cargo test -p parity --release --test tick_late_after_warmup -- --ignored --nocapture
//! ```

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parity::load_parity_env;

/// The frozen v3 world the live runs use.
const V3_TEMPLATE: &str = r"C:\MineRust-data\world-template-v3";
/// Where the report lands.
const TRACE_DIR: &str = r"C:\MineRust-data\traces";
/// How long the idle window lasts.
const IDLE: Duration = Duration::from_secs(20);
/// Sampling period.
const SAMPLE: Duration = Duration::from_secs(1);

/// One sample of the tick metrics.
struct Sample {
    /// Seconds since the tick thread started.
    at_s: f64,
    /// Total ticks published so far.
    ticks: u64,
    /// Total late ticks so far.
    late_ticks: u64,
    /// Worst wake latency so far.
    late_max_ms: f64,
    /// Wake jitter p99 so far.
    jitter_p99_ms: f64,
}

#[tokio::test]
#[ignore = "requires the owner's installation (C:\\MineRust-data)"]
async fn the_tick_does_not_wake_late_without_players() {
    let env = match load_parity_env() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            return;
        }
    };
    if !Path::new(V3_TEMPLATE).is_dir() {
        eprintln!("SKIP: v3 template not found at {V3_TEMPLATE}");
        return;
    }
    let mut config = env.config.clone();
    config.world_dir = Some(V3_TEMPLATE.to_owned());

    // The production order: build the world, read it once, then serve.
    let build_started = Instant::now();
    let status = Arc::new(server::status_info(&config));
    let data = Arc::new(
        data::load_from_jar(
            &config.client_jar,
            config.registries_report.as_deref().map(Path::new),
        )
        .expect("game data from the jar"),
    );
    let (loader, world) = server::build_world(&config, Arc::clone(&data)).expect("v3 builds");
    let built = build_started.elapsed();
    let warm_started = Instant::now();
    if let Some(loader) = &loader {
        let reply = loader.load_batch().await.expect("warm-up queued");
        reply
            .await
            .expect("warm-up reply alive")
            .expect("warm-up read");
    }
    let warmed = warm_started.elapsed();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let ids = Arc::new(env.packet_ids().expect("packet ids"));
    let metrics = Arc::new(net::game::TickMetricsHandle::default());
    let game_config = net::game::GameConfig {
        workers: config.workers,
        region_side: config.region_side,
        ..net::game::GameConfig::default()
    };
    let server = tokio::spawn(net::serve_with_metrics(
        game_config,
        Some(Arc::clone(&metrics)),
        listener,
        status,
        data,
        ids,
        loader,
        world,
    ));
    let tick_started = Instant::now();

    let mut samples: Vec<Sample> = Vec::new();
    while tick_started.elapsed() < IDLE {
        tokio::time::sleep(SAMPLE).await;
        let snapshot = metrics.snapshot();
        samples.push(Sample {
            at_s: tick_started.elapsed().as_secs_f64(),
            ticks: snapshot.ticks,
            late_ticks: snapshot.late_ticks,
            late_max_ms: snapshot.late_max_ms,
            jitter_p99_ms: snapshot.jitter_p99_ms,
        });
    }
    server.abort();

    let mut text = String::from("# M4.5 Шаг 7.4 — тик без игроков после прогрева\n\n");
    text.push_str(&format!(
        "world build: {:.3} s | warm-up read: {:.3} s | idle window: {:.1} s | players: 0\n\n",
        built.as_secs_f64(),
        warmed.as_secs_f64(),
        IDLE.as_secs_f64()
    ));
    // `TickMetricsHandle` is republished every 100 ticks (`PUBLISH_EVERY`,
    // 5 s at 20 TPS), so the columns step instead of moving smoothly: a
    // repeated row means "not republished since", not "the tick stalled".
    text.push_str(
        "Metrics are republished every 100 ticks (5 s at 20 TPS): repeated rows \
         mean \"not republished yet\", not a stalled tick. The `tick woke late` \
         line measures `uptime_ms` from the tick thread start, which `server::run` \
         reaches only after the world build and the warm-up read.\n\n",
    );
    text.push_str("| at s | ticks | late_ticks | late_max ms | jitter p99 ms |\n");
    text.push_str("|---:|---:|---:|---:|---:|\n");
    for sample in &samples {
        text.push_str(&format!(
            "| {:.1} | {} | {} | {:.3} | {:.3} |\n",
            sample.at_s, sample.ticks, sample.late_ticks, sample.late_max_ms, sample.jitter_p99_ms
        ));
    }
    let mut first_late: Option<&Sample> = None;
    let mut previous = 0;
    for sample in &samples {
        if sample.late_ticks > previous {
            first_late = Some(sample);
            break;
        }
        previous = sample.late_ticks;
    }
    let total_late = samples.last().map(|s| s.late_ticks).unwrap_or(0);
    let worst = samples.last().map(|s| s.late_max_ms).unwrap_or(0.0);
    text.push_str(&format!(
        "\nfirst late tick: {}\ntotal late ticks: {total_late}\nworst late_ms: {worst:.3}\n",
        match first_late {
            Some(sample) => format!(
                "at {:.1} s (tick {}, late_max {:.3} ms)",
                sample.at_s, sample.ticks, sample.late_max_ms
            ),
            None => "none".to_owned(),
        }
    ));
    let path = Path::new(TRACE_DIR).join("tick-idle-after-warmup.md");
    if let Err(error) = std::fs::create_dir_all(TRACE_DIR) {
        eprintln!("SKIP: cannot create {TRACE_DIR}: {error}");
        return;
    }
    if let Err(error) = std::fs::write(&path, &text) {
        eprintln!("cannot write {}: {error}", path.display());
    }
    for sample in &samples {
        eprintln!(
            "at {:.0}s ticks={} late={} late_max={:.3} jitter_p99={:.3}",
            sample.at_s, sample.ticks, sample.late_ticks, sample.late_max_ms, sample.jitter_p99_ms
        );
    }
    eprintln!(
        "build {:.3}s warm {:.3}s total late {total_late} worst {worst:.3} — report {}",
        built.as_secs_f64(),
        warmed.as_secs_f64(),
        path.display()
    );
    assert!(
        total_late == 0,
        "the tick woke late {total_late} times with no player (worst {worst:.3} ms); \
         see {}",
        path.display()
    );
}
