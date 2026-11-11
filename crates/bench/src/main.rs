//! The M4.6 benchmark CLI (ARCH-M4 §5): what the region pool mechanism buys
//! and what it costs, measured on the real `Scheduler`/`Region` code without
//! any network.
//!
//! Modes:
//!
//! * `capacity` — the scheduler called in a loop with no tick-wait pause;
//!   tick duration percentiles and ticks/s.
//! * `paced` — the real 20 TPS absolute-deadline loop; drift, jitter, late
//!   ticks and missed slots.
//!
//! Full-matrix series are run manually (D6, `docs/BENCH-M4.md`); CI runs
//! only the smoke test (`cargo test -p bench`).

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "help".to_owned());
    match mode.as_str() {
        "capacity" => bench::capacity::run_cli(args.next().as_deref()),
        "paced" => bench::paced::run_cli(args.next().as_deref()),
        other => anyhow::bail!(
            "usage: bench <capacity|paced> [label]\n  \
             the label names the report file in target/bench/ (got: {other})"
        ),
    }
}
