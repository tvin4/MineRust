//! The M4.6 benchmark CLI (ARCH-M4 §5): what the region pool mechanism buys
//! and what it costs, measured on the real `Scheduler`/`Region` code.
//!
//! Modes:
//!
//! * `capacity` — the scheduler called in a loop with no tick-wait pause;
//!   tick duration percentiles and ticks/s.
//! * `paced` — the real 20 TPS absolute-deadline loop; drift, jitter, late
//!   ticks and missed slots.
//! * `streaming` — one bench bot on the synthetic world, straight walk at
//!   1.0 block/tick, 120+ chunk crossings (the M4.4 criterion).
//! * `load N K SIDE SECONDS` — N bots walk SECONDS seconds on an in-process
//!   server with K workers and SIDE-chunk region tiles.
//! * `phase` — the Шаг 0.2 series: join phase vs steady walking for
//!   N=100/K=6/side=16 and N=200/K=10/side=16, 3 repeats (BENCH-M4 §5).
//! * `visibility` — the M4.5 Шаг 3.5 series: the capacity engine on the
//!   dense and uniform player geometries, P=64/200/512, 3 repeats.
//! * `visibility-phases` — Шаг 3.5 per-phase timing: the same geometries with
//!   the tick split into visibility pass, packet build, delivery and the rest.
//!
//! Full-matrix series are run manually (D6, `docs/BENCH-M4.md`); CI runs
//! only the smoke test (`cargo test -p bench`).

use bench::network;

fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "help".to_owned());
    match mode.as_str() {
        "capacity" => bench::capacity::run_cli(args.next().as_deref()),
        "paced" => bench::paced::run_cli(args.next().as_deref()),
        "streaming" => runtime.block_on(async {
            let version = network::protocol_version()?;
            let report = network::streaming(version).await?;
            let path = network::save_report("streaming", &report)?;
            println!("{report}\nreport: {}", path.display());
            Ok(())
        }),
        "load" => runtime.block_on(async {
            let version = network::protocol_version()?;
            let mut parse = |what: &str| -> anyhow::Result<u64> {
                args.next()
                    .ok_or_else(|| anyhow::anyhow!("load needs {what}"))
                    .and_then(|v| v.parse().map_err(|e| anyhow::anyhow!("{what}: {e}")))
            };
            let (bots, workers, side, seconds) = (
                usize::try_from(parse("bots")?)?,
                usize::try_from(parse("workers")?)?,
                i32::try_from(parse("region_side")?)?,
                parse("seconds")?,
            );
            let report = network::load(bots, workers, side, seconds, version).await?;
            let path = network::save_report("load", &report)?;
            println!("{report}\nreport: {}", path.display());
            Ok(())
        }),
        "priority" => runtime.block_on(async {
            let version = network::protocol_version()?;
            let report = network::priority_ab(100, 20, 3, version).await?;
            let path = network::save_report("priority", &report)?;
            println!("{report}\nreport: {}", path.display());
            Ok(())
        }),
        "phase" => runtime.block_on(async {
            let version = network::protocol_version()?;
            network::phase_cli(version).await.map(|_| ())
        }),
        "visibility" => runtime.block_on(async { bench::visibility::run_cli().map(|_| ()) }),
        "visibility-phases" => {
            runtime.block_on(async { bench::visibility::run_phases_cli().map(|_| ()) })
        }
        other => anyhow::bail!(
            "usage: bench <capacity|paced|streaming|load|priority> ...\n  \
             capacity/paced [label]\n  \
             load <bots> <workers> <region_side> <seconds>\n  \
             priority (A3: 100 bots + 8 busy threads, 3 repeats per half)\n  \
             visibility (M4.5 Шаг 3.5: dense/uniform geometries, P=64/200/512)\n  \
             visibility-phases (M4.5 Шаг 3.5: per-phase tick timing)\n  \
             the label names the report file in target/bench/ (got: {other})"
        ),
    }
}
