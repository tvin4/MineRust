//! Pacing of chunk bursts: how fast the client wants them to arrive.
//!
//! After `chunk_batch_finished` the client answers with the serverbound
//! `minecraft:chunk_batch_received` — one float, the chunks it wants per
//! game tick — and the bursts that follow are split to that rate, one
//! sub-batch per tick (step 3.3d).
//!
//! SOURCE: wiki, unverified — the singles-float composition and its
//! meaning are the documented ones, never byte-proven against a vanilla
//! server; the parity test `chunk_batch_slice` is the check (its result
//! goes to `docs/SOURCES.md`).

use std::time::Duration;

/// Length of one game tick, the unit the pace is counted in.
// SOURCE: wiki, unverified (vanilla ticks twenty times a second; nothing
// in the recordings measured this interval).
pub(super) const TICK: Duration = Duration::from_millis(50);

/// Fastest pace kept: a client claiming more chunks per tick than this
/// simply wants the next burst in one piece. The bound is a guard against
/// nonsense numbers, not a measured limit.
// SOURCE: wiki, unverified.
const FASTEST: f32 = 1024.0;

/// The pace of one play connection: the rate last confirmed by the client.
///
/// Before the first answer there is no pace at all, and a burst goes out
/// whole — exactly as the vanilla recording frames it (one start, nine
/// chunk payloads, one finished: no gaps in between). From then on every
/// burst wider than the wanted rate is split into sub-batches of
/// [`TICK`] apart.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct ChunkPace {
    /// Chunks per tick the client last confirmed, already clamped.
    wanted: Option<f32>,
}

impl ChunkPace {
    /// The client answered `chunk_batch_received` with its own number.
    ///
    /// Anything that is not a finite value of at least one chunk per tick
    /// keeps the previous pace — a broken claim must never stall or flood
    /// a connection.
    pub(super) fn ack(&mut self, chunks_per_tick: f32) {
        if chunks_per_tick.is_finite() && chunks_per_tick >= 1.0 {
            self.wanted = Some(chunks_per_tick.min(FASTEST));
        } else {
            tracing::warn!(
                chunks_per_tick,
                "implausible chunk pace, keeping the previous one"
            );
        }
    }

    /// Whether the client has confirmed a pace yet (`None` until the
    /// first `chunk_batch_received` arrives).
    pub(super) fn known(&self) -> bool {
        self.wanted.is_some()
    }

    /// How many chunks of a `burst`-sized burst go out per sub-batch;
    /// `None` means "one piece" (no answer yet, or the whole burst fits
    /// into one tick).
    pub(super) fn sub_batch_size(&self, burst: usize) -> Option<usize> {
        if burst == 0 {
            return None;
        }
        let wanted = self.wanted?;
        // `ack` keeps the value finite and inside `[1, FASTEST]`, so the
        // rounding-up cast below is always a small whole number.
        let per_tick = wanted.ceil() as usize;
        let size = per_tick.clamp(1, burst);
        (size < burst).then_some(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without an answer the burst goes out as the recording frames it.
    #[test]
    fn before_the_first_answer_a_burst_is_sent_whole() {
        let pace = ChunkPace::default();
        assert_eq!(pace.sub_batch_size(9), None, "one start/finished pair");
        assert_eq!(pace.sub_batch_size(1), None, "a single chunk as well");
    }

    #[test]
    fn a_confirmed_rate_splits_the_burst_into_tick_sized_groups() {
        let mut pace = ChunkPace::default();
        pace.ack(3.0);
        assert_eq!(pace.sub_batch_size(9), Some(3));
        assert_eq!(pace.sub_batch_size(4), Some(3));
        // A rate wider than the burst itself needs no splitting.
        assert_eq!(pace.sub_batch_size(2), None);
        // A fractional rate is rounded up: never send more than wanted.
        pace.ack(2.5);
        assert_eq!(pace.sub_batch_size(9), Some(3));
        // One chunk per tick, the slowest pace kept.
        pace.ack(1.0);
        assert_eq!(pace.sub_batch_size(4), Some(1));
    }

    #[test]
    fn a_nonsense_answer_never_changes_the_pace() {
        let mut pace = ChunkPace::default();
        pace.ack(f32::NAN);
        assert_eq!(pace.sub_batch_size(9), None, "still no pace");
        pace.ack(4.0);
        assert_eq!(pace.sub_batch_size(9), Some(4), "pace accepted");

        pace.ack(f32::INFINITY);
        assert_eq!(pace.sub_batch_size(9), Some(4), "kept the sane rate");
        pace.ack(0.0);
        assert_eq!(pace.sub_batch_size(9), Some(4), "kept the sane rate");
        pace.ack(-8.0);
        assert_eq!(pace.sub_batch_size(9), Some(4), "kept the sane rate");
        // An absurdly fast claim collapses into "one piece", not a stall.
        pace.ack(1e9);
        assert_eq!(pace.sub_batch_size(9), None, "burst sent in one piece");
    }

    /// The pace of the tick constant: spaced by a whole game tick each.
    #[test]
    fn sub_batches_are_one_game_tick_apart() {
        assert_eq!(TICK, Duration::from_millis(50), "twenty ticks a second");
        assert_eq!(FASTEST, 1024.0, "the guard bound of a wild claim");
    }
}
