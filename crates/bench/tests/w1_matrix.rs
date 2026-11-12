//! The W1 repeat matrix of M4.5 Шаг 0.3 (BENCH-M4 §1): the real tick with no
//! synthetic load, K ∈ {0,2,6,10,12}, three repeats, min/median/max, on the
//! corrected multi-region layout (the old W1 table was one run on a layout
//! that put all players in one region, so K did not mean anything).
//!
//! Manual (D6): `cargo test -p bench --release --test w1_matrix -- --ignored
//! --nocapture`. Writes `target/bench/m45-w1-matrix.md`.

use bench::players::{set_step, spread_over_tiles};
use bench::{BenchSpec, Workload};

/// min/median/max of exactly three samples.
fn min_med_max(mut values: Vec<f64>) -> (f64, f64, f64) {
    values.sort_by(|a, b| a.partial_cmp(b).expect("finite tick times"));
    (values[0], values[1], values[2])
}

#[test]
#[ignore]
fn w1_matrix() {
    let ks = [0usize, 2, 6, 10, 12];
    let repeats = 3;
    let mut markdown = String::from(
        "# W1 — real tick, no synthetic load (M4.5 Шаг 0.3)\n\n\
         region_side = 64, 64 players walking 0.25 blocks/tick over 64 region \
         tiles, warm-up 100 ticks + 200 measured, 3 repeats.\n\
         p50 is min/median/max over the 3 repeats; speedup is median vs K=0.\n\n\
         | K | p50 min/med/max μs | p99 med μs | max med μs | tps med | speedup vs K=0 |\n\
         |---|---|---|---|---|---|\n",
    );
    let mut baseline = 0.0f64;
    for &k in &ks {
        let mut p50s = Vec::new();
        let mut p99s = Vec::new();
        let mut maxs = Vec::new();
        let mut tpss = Vec::new();
        for _ in 0..repeats {
            let spec = BenchSpec {
                workers: k,
                region_side: 64,
                players: 64,
                workload: Workload::W1,
                warmup: 100,
                ticks: 200,
                seconds: 0,
            };
            let mut players = spread_over_tiles(64, 64, 8, (0, 0));
            set_step(&mut players, 0.25);
            let scheduler = bench::capacity::build(spec, &mut players).expect("build");
            let result = bench::capacity::run(scheduler, &mut players, spec).expect("run");
            p50s.push(result.tick_us[0]);
            p99s.push(result.tick_us[1]);
            maxs.push(result.tick_us[3]);
            tpss.push(result.tps);
        }
        let (min, med, max) = min_med_max(p50s);
        let p99 = min_med_max(p99s).1;
        let worst = min_med_max(maxs).1;
        let tps = min_med_max(tpss).1;
        if k == 0 {
            baseline = med;
        }
        let speedup = if med > 0.0 { baseline / med } else { 0.0 };
        markdown.push_str(&format!(
            "| {k} | {min:.0} / {med:.0} / {max:.0} | {p99:.0} | {worst:.0} | {tps:.0} | {speedup:.2}× |\n"
        ));
        println!(
            "K={k}: p50 {min:.0}/{med:.0}/{max:.0} μs, p99 {p99:.0}, max {worst:.0}, tps {tps:.0}, speedup {speedup:.2}x"
        );
    }
    let dir = bench::report_dir();
    std::fs::create_dir_all(&dir).expect("report dir");
    let path = dir.join("m45-w1-matrix.md");
    std::fs::write(&path, &markdown).expect("write report");
    println!("\nreport: {}", path.display());
}
