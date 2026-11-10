//! Sending one join burst as sub-batches: framing, the first flush and
//! the paced tail.
//!
//! The batch is written one sub-batch per turn of the play loop's
//! `select!` ([`wait_turn`] → [`PendingBatch::send_next`]), never in one
//! sitting: the alternative — writing the whole burst inside the loading
//! branch, as this used to — folds the read side of the loop into the
//! write, so the client's `chunk_batch_received` only lands after the
//! burst has already gone out (the owner's symptom: one sub-batch, 441
//! chunks at once, the pace answer in the end).

use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use protocol::{ChunkBatchFinished, ChunkBatchStart, LevelChunkWithLight, ProtocolState};
use tokio::io::AsyncWrite;
use tokio::time::Instant;
use tracing::{debug, info};

use crate::chunks::{window, ReadyChunks};
use crate::error::ConnectionError;
use crate::frame::write_tracked;

use super::chunk_pace::{ChunkPace, TICK};

/// Chunks of the sub-batch that opens a burst, before the client has
/// named its pace.
///
/// SOURCE: our vanilla 26.3 trace on template v2
/// (`target/parity/chunk-batch-vanilla.json`): the join burst of 72
/// chunks opens with a flush of exactly these 9 — the 3x3 square around
/// the join centre, centre first — and the client's pace answer follows
/// that first `chunk_batch_finished`, never precedes it. The opening is
/// therefore never the whole burst: 9 of 72 went first, the rest only
/// once the pace was known.
const FIRST_FLUSH: usize = 9;

/// How long the burst waits for the client's pace answer after the first
/// sub-batch before sending on regardless.
///
/// Not a protocol value — an internal guard. The recorded answer follows
/// the finished marker immediately, but a client that never answers (a
/// test bot, a silent tool) would stall the join forever without this
/// bound, and the pace-less tail then goes out in one piece like before.
const PACE_WAIT: Duration = Duration::from_secs(1);

/// Where a burst is between its sub-batches; see [`PendingBatch::stage`].
#[derive(Debug, Clone, Copy)]
pub(super) enum Stage {
    /// The first sub-batch has not been written yet — the turn is now.
    First,
    /// The first sub-batch is out; the pace answer may still arrive
    /// until `deadline`, after which the tail goes out pace-less.
    AwaitPace {
        /// Last instant the client may still name its pace.
        deadline: Instant,
    },
    /// The pace is known (or given up on): the next sub-batch is due at
    /// `at` — one game tick after the previous one.
    Pace {
        /// When the next sub-batch becomes due.
        at: Instant,
    },
}

/// One join burst on its way to the wire.
///
/// Created by the play loop when the loader replies and driven from there
/// one sub-batch per turn, so between two sub-batches the loop keeps
/// reading: keep-alives are answered and the client's pace, once it
/// arrives, both stops the wait of [`Stage::AwaitPace`] and sizes every
/// sub-batch after it.
pub(super) struct PendingBatch {
    /// The shared ready world (part A 2); payloads are borrowed on send,
    /// never copied. The window is resolved into it through its index.
    chunks: Arc<ReadyChunks>,
    /// Positions into [`ReadyChunks::chunks`] inside the send order: the
    /// join window (M4.4 A) with the coordinates the world does not hold
    /// skipped, in the distance order.
    order: Vec<usize>,
    /// Next index of `order` to send.
    next: usize,
    /// Chunks written so far (the count of the whole burst's log line).
    sent: i32,
    /// Sub-batches written so far.
    batches: u32,
    /// Chunk cache centre the framing counts against.
    center_x: i32,
    /// Chunk cache centre, second coordinate.
    center_z: i32,
    /// Where the burst stands between sub-batches.
    stage: Stage,
}

impl PendingBatch {
    /// Builds the send order from the join window (M4.4 A): the window
    /// coordinates around the centre at this radius, resolved O(1) through
    /// the ready index, with the coordinates the world does not hold
    /// skipped (a join at a world edge sends what exists, never a hole).
    /// The order then puts the player's chunk first and the rest by growing
    /// squared distance.
    pub(super) fn new(ready: Arc<ReadyChunks>, center_x: i32, center_z: i32, radius: i32) -> Self {
        let wanted = window(center_x, center_z, radius);
        let mut order: Vec<usize> = wanted
            .iter()
            .filter_map(|&(chunk_x, chunk_z)| ready.position(chunk_x, chunk_z))
            .collect();
        let not_cached = wanted.len() - order.len();
        info!(
            radius,
            center_x,
            center_z,
            in_window = wanted.len(),
            sent = order.len(),
            not_cached,
            "chunk window selected"
        );
        order.sort_unstable_by_key(|&index| {
            let chunk = &ready.chunks()[index];
            order_key(chunk.chunk_x, chunk.chunk_z, center_x, center_z)
        });
        Self {
            chunks: ready,
            order,
            next: 0,
            sent: 0,
            batches: 0,
            center_x,
            center_z,
            stage: Stage::First,
        }
    }

    /// The stage the next turn of the loop has to wait out (a copy, so
    /// the loop can hold `&mut self` while it waits).
    pub(super) fn stage(&self) -> Stage {
        self.stage
    }

    /// The client named its pace: a burst waiting for exactly that answer
    /// starts its tail right now — the vanilla trace sends its second
    /// flush directly behind the answer, without spending a tick on the
    /// wait.
    pub(super) fn on_paced(&mut self) {
        if matches!(self.stage, Stage::AwaitPace { .. }) {
            self.stage = Stage::Pace { at: Instant::now() };
        }
    }

    /// Writes the next sub-batch: `Ok(true)` when the burst is complete
    /// (the play loop drops it then), `Ok(false)` when more is left and
    /// the caller must wait out [`Self::stage`] before asking again.
    pub(super) async fn send_next<W>(
        &mut self,
        writer: &mut W,
        ids: &PacketIds,
        pace: &ChunkPace,
    ) -> Result<bool, ConnectionError>
    where
        W: AsyncWrite + Unpin,
    {
        let left = self.order.len() - self.next;
        if left == 0 {
            // An empty read still frames its own (empty) batch.
            write_sub_batch(writer, ids, &[], self.center_x, self.center_z).await?;
            self.batches = 1;
            info!(chunk_count = 0, sub_batches = 1, "chunk batch sent");
            return Ok(true);
        }
        // The size of this sub-batch: the rate the client confirmed,
        // while the first flush before any answer stays within
        // [`FIRST_FLUSH`] — the opening of the recorded vanilla burst.
        let size = match self.stage {
            Stage::First if !pace.known() => left.min(FIRST_FLUSH),
            _ => pace.sub_batch_size(left).unwrap_or(left),
        };
        let group: Vec<&LevelChunkWithLight> = self.order[self.next..self.next + size]
            .iter()
            .map(|&index| &self.chunks.chunks()[index])
            .collect();
        self.next += size;
        self.sent += write_sub_batch(writer, ids, &group, self.center_x, self.center_z).await?;
        self.batches += 1;
        if self.next == self.order.len() {
            info!(
                chunk_count = self.sent,
                sub_batches = self.batches.max(1),
                "chunk batch sent"
            );
            return Ok(true);
        }
        let now = Instant::now();
        self.stage = match self.stage {
            // The first sub-batch is out; an answer may already have
            // arrived while it was writing.
            Stage::First if pace.known() => Stage::Pace { at: now + TICK },
            Stage::First => Stage::AwaitPace {
                deadline: now + PACE_WAIT,
            },
            // Tail sub-batches are one game tick apart, whether the pace
            // arrived or the client stayed silent.
            _ => Stage::Pace { at: now + TICK },
        };
        Ok(false)
    }
}

/// Waits out one stage of a burst: nothing for the first sub-batch, a
/// deadline for the pace answer, a game tick for the paced tail.
///
/// `None` (no burst pending) parks forever — the guarded `select!` arm
/// of the play loop never polls it then.
pub(super) async fn wait_turn(stage: Option<Stage>) -> Option<()> {
    match stage? {
        Stage::First => {}
        Stage::AwaitPace { deadline } => tokio::time::sleep_until(deadline).await,
        Stage::Pace { at } => tokio::time::sleep_until(at).await,
    }
    Some(())
}

/// Writes one whole burst from a caller with no play loop of its own —
/// the unit tests, which have no incoming packets to service between
/// sub-batches. The loop itself drives [`PendingBatch::send_next`]
/// directly so the client's answer can land in between.
#[cfg(test)]
pub(super) async fn write_chunk_batch<W>(
    writer: &mut W,
    ids: &PacketIds,
    ready: Arc<ReadyChunks>,
    pace: &ChunkPace,
    center_x: i32,
    center_z: i32,
    radius: i32,
) -> Result<(), ConnectionError>
where
    W: AsyncWrite + Unpin,
{
    let mut batch = PendingBatch::new(ready, center_x, center_z, radius);
    loop {
        if batch.send_next(writer, ids, pace).await? {
            return Ok(());
        }
        wait_turn(Some(batch.stage())).await;
    }
}

/// Writes one `chunk_batch_start` ... `chunk_batch_finished` pair around
/// `chunks`, with the finished count equal to what was sent in between.
///
/// Every chunk is logged at debug with its coordinates and payload size —
/// info keeps only the batch summaries (`chunk_batch_finished sent`), so
/// one join's worth of bursts does not print a line per payload at the
/// default level (part A 2/3: 289 chunks were 289 info lines each). The
/// sub-batch arriving here was already cut to the join window by
/// [`PendingBatch::new`] (M4.4 A), so every chunk fits the shape around
/// `center` the join sends.
async fn write_sub_batch<W>(
    writer: &mut W,
    ids: &PacketIds,
    chunks: &[&LevelChunkWithLight],
    center_x: i32,
    center_z: i32,
) -> Result<i32, ConnectionError>
where
    W: AsyncWrite + Unpin,
{
    debug!(center_x, center_z, "chunk_batch_start sent");
    write_tracked(
        writer,
        ProtocolState::Play,
        ids.id(ChunkBatchStart::KEY),
        &ChunkBatchStart.encode_payload(),
    )
    .await?;
    let mut chunk_count = 0_i32;
    for chunk in chunks {
        let payload = chunk.encode_payload()?;
        debug!(
            chunk_x = chunk.chunk_x,
            chunk_z = chunk.chunk_z,
            size = payload.len(),
            "level_chunk_with_light sent"
        );
        write_tracked(
            writer,
            ProtocolState::Play,
            ids.id(LevelChunkWithLight::KEY),
            &payload,
        )
        .await?;
        chunk_count += 1;
    }
    let finished = ChunkBatchFinished { chunk_count };
    write_tracked(
        writer,
        ProtocolState::Play,
        ids.id(ChunkBatchFinished::KEY),
        &finished.encode_payload(),
    )
    .await?;
    info!(chunk_count, "chunk_batch_finished sent");
    Ok(chunk_count)
}

/// Puts a burst in the order the client must receive it: the player's own
/// chunk first and then by growing squared chunk distance, ties broken by
/// `(chunk_x, chunk_z)` so one world of ready chunks always sends the same
/// sequence.
///
/// SOURCE: our vanilla 26.3 traces on template v2. In
/// `target/parity/chunk-batch-vanilla.json` every one of the 11 flushes of
/// the 72 recorded chunks has a *non-decreasing* squared distance from the
/// join centre (6, -2), and the first flush is exactly the 3x3 square
/// around it (9 chunks, centre first, then the four orthogonal neighbours,
/// then the four corners). `target/parity/join-vanilla.v2runB.json` carries
/// the same nine chunks in the same byte order. The whole trace shows
/// five distance regressions, all of them on a flush boundary: a flush
/// sorts the chunks it already has while the rest are still loading — so
/// what one burst can copy is the distance order inside a flush, not a
/// global order across flushes. Vanilla's order *within* one distance (its
/// first four neighbours go N, E, W, S) is stable across runs but its key
/// is never visible on the wire, and the traces do not cover a tie deeper
/// than the 3x3 — hence the coordinate tie-break above, which reproduces
/// the observed grouping (centre, four neighbours, four corners) with a
/// rule of our own.
pub(super) fn order_key(
    chunk_x: i32,
    chunk_z: i32,
    center_x: i32,
    center_z: i32,
) -> (i64, i32, i32) {
    let delta_x = i64::from(chunk_x - center_x);
    let delta_z = i64::from(chunk_z - center_z);
    (delta_x * delta_x + delta_z * delta_z, chunk_x, chunk_z)
}

/// The same rule over borrowed payloads — what [`PendingBatch`] applies
/// to its indices, in the shape the ordering test reads back.
#[cfg(test)]
pub(super) fn sort_by_distance(chunks: &mut [&LevelChunkWithLight], center_x: i32, center_z: i32) {
    chunks
        .sort_unstable_by_key(|chunk| order_key(chunk.chunk_x, chunk.chunk_z, center_x, center_z));
}

/// What stays true of this module's numbers: the tick of the paced tail
/// and the guard bound of a pace answer that never comes.
#[cfg(test)]
mod tests {
    use super::{PACE_WAIT, TICK};

    #[test]
    fn the_sub_batch_spacing_and_the_answer_guard() {
        assert_eq!(TICK, std::time::Duration::from_millis(50), "one tick");
        assert_eq!(
            PACE_WAIT,
            std::time::Duration::from_secs(1),
            "the silent-client guard"
        );
    }
}
