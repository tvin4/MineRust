//! The W2 matrix of M4.5 Шаг 0 (BENCH-M4 §2): the scalability ceiling of the
//! region pool under a synthetic per-region load, after the Шаг 0.1а bench
//! defect was fixed (the old `spread` put every player in one region at
//! `region_side = 64`, so the pool never engaged).
//!
//! Each configuration is a time-bounded capacity run: 5 s warm-up, then 10 s
//! sampled, 3 repeats; players stand still so that the number of regions with
//! players — and therefore the number of busy-loops — stays constant.
//!
//! Manual (D6): `cargo test -p bench --release --test w2_matrix -- --ignored
//! --nocapture`. Writes `target/bench/m45-w2-matrix.md`.

use std::time::Duration;

use bench::players::spread_over_tiles;
use bench::{BenchSpec, Workload};

/// min/median/max of exactly three samples.
fn min_med_max(mut values: Vec<f64>) -> (f64, f64, f64) {
    values.sort_by(|a, b| a.partial_cmp(b).expect("finite tick times"));
    (values[0], values[1], values[2])
}

#[test]
#[ignore]
fn w2_matrix() {
    let ks = [0usize, 1, 2, 4, 6, 10, 12];
    let region_counts = [4usize, 16, 64];
    let loads = [50u64, 200, 1000];
    let repeats = 3;
    let warmup = Duration::from_secs(5);
    let measure = Duration::from_secs(10);

    let mut markdown = String::from(
        "# W2 matrix — M4.5 Шаг 0 (region pool scalability ceiling)\n\n\
         region_side = 64, 64 players (standing still), 5 s warm-up + 10 s \
         measured, 3 repeats.\n\
         p50 is min/median/max over the 3 repeats; p99/tps are medians; \
         speedup is median p50(K=0) / median p50(K).\n\n\
         | S μs | regions | K | p50 min/med/max μs | p99 med μs | tps med | speedup vs K=0 |\n\
         |---|---|---|---|---|---|---|\n",
    );

    for &s in &loads {
        for &regions in &region_counts {
            let axis = (regions as f64).sqrt().round() as i32;
            let mut baseline_med = 0.0f64;
            for &k in &ks {
                let mut p50s = Vec::with_capacity(repeats);
                let mut p99s = Vec::with_capacity(repeats);
                let mut tpss = Vec::with_capacity(repeats);
                for _ in 0..repeats {
                    let spec = BenchSpec {
                        workers: k,
                        region_side: 64,
                        players: 64,
                        workload: Workload::W2(s),
                        warmup: 0,
                        ticks: 0,
                        seconds: 0,
                    };
                    let mut players = spread_over_tiles(64, 64, axis, (0, 0));
                    let scheduler =
                        bench::capacity::build(spec, &mut players).expect("scheduler builds");
                    let result =
                        bench::capacity::run_timed(scheduler, &mut players, warmup, measure)
                            .expect("timed run measures");
                    p50s.push(result.tick_us[0]);
                    p99s.push(result.tick_us[1]);
                    tpss.push(result.tps);
                }
                let (min, med, max) = min_med_max(p50s);
                let p99 = min_med_max(p99s).1;
                let tps = min_med_max(tpss).1;
                if k == 0 {
                    baseline_med = med;
                }
                let speedup = if med > 0.0 { baseline_med / med } else { 0.0 };
                markdown.push_str(&format!(
                    "| {s} | {regions} | {k} | {min:.0} / {med:.0} / {max:.0} | {p99:.0} | {tps:.0} | {speedup:.2}× |\n"
                ));
                println!(
                    "S={s:>4} regions={regions:>2} K={k:>2}: p50 {min:.0}/{med:.0}/{max:.0} μs, \
                     p99 {p99:.0} μs, tps {tps:.0}, speedup {speedup:.2}x"
                );
            }
        }
    }

    let dir = bench::report_dir();
    std::fs::create_dir_all(&dir).expect("report dir");
    let path = dir.join("m45-w2-matrix.md");
    std::fs::write(&path, &markdown).expect("write report");
    println!("\nreport: {}", path.display());
}
