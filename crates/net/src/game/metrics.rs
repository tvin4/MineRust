//! Timing metrics of the game thread.
//!
//! Two numbers per tick: how long the tick's work took ("tick duration")
//! and how far the thread woke from its absolute deadline (jitter). Both
//! feed the M4.1 criteria (drift < 5 ms, jitter p99 < 20 ms) and the log
//! line the live check reads.
//!
//! The game thread keeps the samples locally and publishes percentiles
//! through atomics, so reading them never blocks the tick.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Recompute and publish the percentiles every this many ticks.
const PUBLISH_EVERY: u64 = 100;

/// A tick whose wake latency exceeds this many milliseconds is "late": it is
/// logged (one `warn` line) and counted in [`TickMetrics::late_ticks`].
pub(crate) const LATE_TICK_MS: u64 = 20;

/// Samples kept for the percentiles (one hour of 20 TPS ticks).
const MAX_SAMPLES: usize = 72_000;

/// A snapshot of the game thread's timing, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TickMetrics {
    /// Number of ticks recorded so far.
    pub ticks: u64,
    /// `elapsed − ticks · period`, the accumulated deadline error.
    pub drift_ms: f64,
    /// Median wake latency (how late the thread woke), in ms.
    pub jitter_p50_ms: f64,
    /// 99th percentile wake latency, in ms.
    pub jitter_p99_ms: f64,
    /// Worst wake latency, in ms.
    pub jitter_max_ms: f64,
    /// Median tick duration (work time), in ms.
    pub duration_p50_ms: f64,
    /// 99th percentile tick duration, in ms.
    pub duration_p99_ms: f64,
    /// Worst tick duration, in ms.
    pub duration_max_ms: f64,
    /// Number of ticks that woke more than [`LATE_TICK_MS`] late.
    pub late_ticks: u64,
    /// Worst wake latency among those late ticks, in ms.
    pub late_max_ms: f64,
}

/// The atomics the game thread publishes [`TickMetrics`] through.
#[derive(Debug, Default)]
pub struct TickMetricsHandle {
    ticks: AtomicU64,
    drift_us: AtomicI64,
    jitter_p50_us: AtomicU64,
    jitter_p99_us: AtomicU64,
    jitter_max_us: AtomicU64,
    duration_p50_us: AtomicU64,
    duration_p99_us: AtomicU64,
    duration_max_us: AtomicU64,
    late_ticks: AtomicU64,
    late_max_us: AtomicU64,
}

impl TickMetricsHandle {
    /// Reads the last published values.
    pub fn snapshot(&self) -> TickMetrics {
        TickMetrics {
            ticks: self.ticks.load(Ordering::Relaxed),
            drift_ms: micros_to_ms_i64(self.drift_us.load(Ordering::Relaxed)),
            jitter_p50_ms: micros_to_ms(self.jitter_p50_us.load(Ordering::Relaxed)),
            jitter_p99_ms: micros_to_ms(self.jitter_p99_us.load(Ordering::Relaxed)),
            jitter_max_ms: micros_to_ms(self.jitter_max_us.load(Ordering::Relaxed)),
            duration_p50_ms: micros_to_ms(self.duration_p50_us.load(Ordering::Relaxed)),
            duration_p99_ms: micros_to_ms(self.duration_p99_us.load(Ordering::Relaxed)),
            duration_max_ms: micros_to_ms(self.duration_max_us.load(Ordering::Relaxed)),
            late_ticks: self.late_ticks.load(Ordering::Relaxed),
            late_max_ms: micros_to_ms(self.late_max_us.load(Ordering::Relaxed)),
        }
    }
}

fn micros_to_ms(micros: u64) -> f64 {
    micros as f64 / 1000.0
}

fn micros_to_ms_i64(micros: i64) -> f64 {
    micros as f64 / 1000.0
}

/// The per-run sample buffer of the game thread.
#[derive(Debug)]
pub(crate) struct Recorder {
    start: Instant,
    period: Duration,
    jitter: VecDeque<u64>,
    duration: VecDeque<u64>,
    ticks: u64,
    late_ticks: u64,
    late_max_us: u64,
    last_woke: Instant,
}

impl Recorder {
    /// A recorder measuring against `start = now`.
    pub(crate) fn new(period: Duration) -> Self {
        let now = Instant::now();
        Self {
            start: now,
            period,
            jitter: VecDeque::new(),
            duration: VecDeque::new(),
            ticks: 0,
            late_ticks: 0,
            late_max_us: 0,
            last_woke: now,
        }
    }

    /// Records one tick and publishes the percentiles every
    /// [`PUBLISH_EVERY`] ticks.
    pub(crate) fn record(
        &mut self,
        tick: u64,
        deadline: Instant,
        woke: Instant,
        duration: Duration,
        handle: &TickMetricsHandle,
    ) {
        let late_us = woke.saturating_duration_since(deadline).as_micros() as u64;
        push(&mut self.jitter, late_us);
        push(&mut self.duration, duration.as_micros() as u64);
        if late_us > LATE_TICK_MS * 1000 {
            self.late_ticks += 1;
            self.late_max_us = self.late_max_us.max(late_us);
        }
        self.ticks = tick + 1;
        self.last_woke = woke;
        if self.ticks.is_multiple_of(PUBLISH_EVERY) {
            self.publish(handle);
        }
    }

    /// Publishes the final values (call once when the loop ends).
    pub(crate) fn finish(&self, handle: &TickMetricsHandle) {
        self.publish(handle);
    }

    fn publish(&self, handle: &TickMetricsHandle) {
        let elapsed = self.last_woke.saturating_duration_since(self.start);
        let expected = self.period.mul_f64(self.ticks as f64);
        let drift_us = elapsed.as_micros() as i64 - expected.as_micros() as i64;
        handle.ticks.store(self.ticks, Ordering::Relaxed);
        handle.drift_us.store(drift_us, Ordering::Relaxed);
        let (p50, p99, max) = percentiles(&self.jitter);
        handle.jitter_p50_us.store(p50, Ordering::Relaxed);
        handle.jitter_p99_us.store(p99, Ordering::Relaxed);
        handle.jitter_max_us.store(max, Ordering::Relaxed);
        let (p50, p99, max) = percentiles(&self.duration);
        handle.duration_p50_us.store(p50, Ordering::Relaxed);
        handle.duration_p99_us.store(p99, Ordering::Relaxed);
        handle.duration_max_us.store(max, Ordering::Relaxed);
        handle.late_ticks.store(self.late_ticks, Ordering::Relaxed);
        handle
            .late_max_us
            .store(self.late_max_us, Ordering::Relaxed);
    }
}

/// Keeps at most [`MAX_SAMPLES`] values, dropping the oldest first.
fn push(samples: &mut VecDeque<u64>, value: u64) {
    if samples.len() == MAX_SAMPLES {
        samples.pop_front();
    }
    samples.push_back(value);
}

/// `(p50, p99, max)` of the samples; all zero when there are none.
fn percentiles(samples: &VecDeque<u64>) -> (u64, u64, u64) {
    if samples.is_empty() {
        return (0, 0, 0);
    }
    let mut sorted: Vec<u64> = samples.iter().copied().collect();
    sorted.sort_unstable();
    let last = sorted.len() - 1;
    (sorted[last / 2], sorted[last * 99 / 100], sorted[last])
}
