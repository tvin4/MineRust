//! Tick-wait measurement for ARCH-M4 §3: how late does a 20 TPS loop wake
//! up on this Windows machine, and does the method used for waiting change
//! the answer?
//!
//! Wake latency is `woken - deadline` (the absolute instant tick `i` was
//! due): negative means an early wake, positive means a late one. Every run
//! reports min / median / p99 / p99.9 / max in milliseconds, the drift of
//! the whole run and the five biggest spikes with their tick index, so a
//! one-off outlier can be located instead of averaged away.
//!
//! Two scenarios, as required by the design review: `idle` (nothing else
//! runs) and `load` (busy threads occupy every logical core but one).
//!
//! Modes measured: `deadline-sleep` (sleep until the absolute deadline),
//! `hybrid` (sleep 2 ms ahead of it, then spin) and `tokio::interval`
//! (what the keep-alive of the server uses today). The `probe` at start-up
//! checks whether `std::thread::sleep` on this machine is the high precision
//! variant: it asks for 1/2/5/10 ms and shows what sleep actually costs.
//!
//! Run: cargo run --release --manifest-path tools/tickbench/Cargo.toml

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Tick length of the design: 20 TPS.
const TICK: Duration = Duration::from_millis(50);
/// Ticks per run, the design asks for 1000+.
const TICKS: usize = 1000;
/// How far ahead the hybrid mode hands over to spinning.
const HYBRID_SHIFT: Duration = Duration::from_millis(2);
/// Samples per sleep-probe duration.
const PROBE_SAMPLES: usize = 200;

/// One measurement: per-tick wake latency in microseconds, tagged with the
/// tick index (1-based) so spikes can be located.
struct Sample {
    name: &'static str,
    latencies: Vec<(usize, i64)>,
    drift_ms: i64,
}

/// Percentile of an already sorted vector (nearest rank on a 0..1 quantile).
fn percentile(sorted: &[i64], quantile: f64) -> i64 {
    let idx = ((sorted.len() - 1) as f64 * quantile).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

impl Sample {
    /// Prints the required summary line: min / median / p99 / p99.9 / max.
    fn report(&self, scenario: &str) {
        let mut sorted: Vec<i64> = self.latencies.iter().map(|&(_, l)| l).collect();
        sorted.sort_unstable();
        let ms = |micros: i64| micros as f64 / 1000.0;
        let min = *sorted.first().unwrap_or(&0);
        println!(
            "[{scenario}] {:<15} {:>6} ticks | drift {:+} ms | \
             min {:.3} | median {:.3} | p99 {:.3} | p99.9 {:.3} | max {:.3} ms",
            self.name,
            self.latencies.len(),
            self.drift_ms,
            ms(min),
            ms(percentile(&sorted, 0.5)),
            ms(percentile(&sorted, 0.99)),
            ms(percentile(&sorted, 0.999)),
            ms(*sorted.last().unwrap_or(&0)),
        );
        // The five biggest spikes: tick index and time since start, so an
        // outlier like the 58 ms one can be cross-referenced between runs.
        let mut top: Vec<(i64, usize)> = self.latencies.iter().map(|&(i, l)| (l, i)).collect();
        top.sort_unstable_by_key(|&(latency, _)| std::cmp::Reverse(latency));
        let shown: Vec<String> = top
            .iter()
            .take(5)
            .map(|&(lat, tick)| {
                let at = Duration::from_millis(50 * tick as u64);
                format!("tick {tick} at {at:?}: +{:.3} ms", ms(lat))
            })
            .collect();
        println!(
            "[{scenario}] {:<15} top spikes: {}",
            self.name,
            shown.join("; ")
        );
    }
}

/// Runs the deadline-sleep mode once.
fn run_deadline_sleep() -> Sample {
    let start = Instant::now();
    let mut latencies = Vec::with_capacity(TICKS);
    for tick in 1..=TICKS {
        let deadline = start + TICK * tick as u32;
        let now = Instant::now();
        if deadline > now {
            thread::sleep(deadline - now);
        }
        latencies.push((tick, (Instant::now() - deadline).as_micros() as i64));
    }
    Sample {
        name: "deadline-sleep",
        latencies,
        drift_ms: start.elapsed().as_millis() as i64 - TICKS as i64 * 50,
    }
}

/// Runs the hybrid mode: sleep until `HYBRID_SHIFT` before the deadline,
/// then spin the remaining gap away.
fn run_hybrid() -> Sample {
    let start = Instant::now();
    let mut latencies = Vec::with_capacity(TICKS);
    for tick in 1..=TICKS {
        let deadline = start + TICK * tick as u32;
        let now = Instant::now();
        if deadline > now + HYBRID_SHIFT {
            thread::sleep(deadline - HYBRID_SHIFT - now);
        }
        while Instant::now() < deadline {
            std::hint::spin_loop();
        }
        latencies.push((tick, (Instant::now() - deadline).as_micros() as i64));
    }
    Sample {
        name: "hybrid(-2ms)",
        latencies,
        drift_ms: start.elapsed().as_millis() as i64 - TICKS as i64 * 50,
    }
}

/// Runs the `tokio::time::interval` mode on a one-thread runtime; the
/// first tick of an interval completes immediately and is skipped.
fn run_tokio_interval() -> Sample {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build();
    let Ok(runtime) = runtime else {
        println!("tokio runtime failed to build, skipped");
        return Sample {
            name: "tokio-interval",
            latencies: Vec::new(),
            drift_ms: 0,
        };
    };
    runtime.block_on(async {
        let mut interval = tokio::time::interval(TICK);
        interval.tick().await; // completes immediately
        let base = Instant::now();
        let mut latencies = Vec::with_capacity(TICKS);
        for tick in 1..=TICKS {
            interval.tick().await;
            let deadline = base + TICK * tick as u32;
            latencies.push((tick, (Instant::now() - deadline).as_micros() as i64));
        }
        Sample {
            name: "tokio-interval",
            latencies,
            drift_ms: base.elapsed().as_millis() as i64 - TICKS as i64 * 50,
        }
    })
}

/// Occupies every logical core but one with a busy loop until dropped.
struct LoadGuard {
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl LoadGuard {
    fn spawn(busy_threads: usize) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let threads = (0..busy_threads)
            .map(|i| {
                let stop = Arc::clone(&stop);
                thread::Builder::new()
                    .name(format!("load-{i}"))
                    .spawn(move || {
                        while !stop.load(Ordering::Relaxed) {
                            std::hint::spin_loop();
                        }
                    })
                    .expect("load thread")
            })
            .collect();
        Self { stop, threads }
    }
}

impl Drop for LoadGuard {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

/// Probes `std::thread::sleep` on this machine: page 1 of the standard
/// Windows timer (15.6 ms and friends) asks for 1/2/5/10 ms and reports
/// what sleep actually spent. Values close to the request mean the
/// high precision path is in effect, values around 15 ms mean it is not.
fn probe_sleep() {
    println!("-- probe: what does std::thread::sleep actually cost here --");
    thread::sleep(Duration::from_millis(20)); // warm up the timer state
    for ms in [1u64, 2, 5, 10] {
        let mut samples = Vec::with_capacity(PROBE_SAMPLES);
        for _ in 0..PROBE_SAMPLES {
            let asked = Duration::from_millis(ms);
            let begin = Instant::now();
            thread::sleep(asked);
            samples.push(begin.elapsed().as_micros() as i64);
        }
        samples.sort_unstable();
        let micros = |v: i64| v as f64 / 1000.0;
        println!(
            "sleep({ms} ms): min {:.3} | median {:.3} | p99 {:.3} | max {:.3} ms",
            micros(*samples.first().unwrap_or(&0)),
            micros(percentile(&samples, 0.5)),
            micros(percentile(&samples, 0.99)),
            micros(*samples.last().unwrap_or(&0)),
        );
    }
}

fn main() {
    let cores = std::thread::available_parallelism()
        .map(|c| c.get())
        .unwrap_or(4);
    println!(
        "tickbench: {TICKS} ticks of {TICK:?} | os={} | logical cores={cores}",
        std::env::consts::OS
    );
    probe_sleep();

    let busy_threads = cores.saturating_sub(1);
    for (scenario, load) in [("idle", false), ("load", true)] {
        let _guard = if load {
            println!("-- scenario {scenario}: {busy_threads} busy threads on {cores} cores --");
            Some(LoadGuard::spawn(busy_threads))
        } else {
            println!("-- scenario {scenario}: nothing else runs --");
            None
        };
        run_deadline_sleep().report(scenario);
        run_hybrid().report(scenario);
        run_tokio_interval().report(scenario);
    }
}
