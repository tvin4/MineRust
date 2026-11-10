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

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use protocol::{
    ChunkBatchFinished, ChunkBatchStart, ForgetLevelChunk, LevelChunkWithLight, ProtocolState,
    SetCenterChunk,
};
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
    /// The opening sub-batch has not been written yet — the turn is now.
    First,
    /// The opening is out; the pace answer may still arrive until
    /// `deadline`, after which the tail goes out pace-less.
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
    /// Nothing is queued: park until a window move queues chunks again. The
    /// guarded `select!` arm of the play loop then never polls, so an idle
    /// connection writes nothing and wakes no timer (M4.4 B).
    Parked,
}

/// The session's chunk sender: the join window first and then every window
/// move the tick reports (M4.4 A/B), one sub-batch per turn of the loop.
///
/// It owns the one copy of the streaming bookkeeping:
///
/// * `queue` — the chunk coordinates still to write, in send order (the join
///   window first, every later `add` appended in the order the tick gave);
/// * `sent` — the coordinates already on the wire, so a later move never
///   sends a chunk twice and knows which leaving chunks to forget;
/// * `center` — the client's current cache centre, the reference the window
///   invariant holds against.
///
/// Sharing the queue with the join burst is what lets a window move *cancel*
/// a sub-batch that has not gone out yet: a chunk that left the window while
/// still queued is dropped without a `forget_level_chunk`.
pub(super) struct PendingBatch {
    /// The shared ready world (part A 2); payloads are borrowed on send,
    /// never copied. Coordinates are resolved through its index.
    chunks: Arc<ReadyChunks>,
    /// Coordinates waiting to be written, in send order (front first).
    queue: VecDeque<(i32, i32)>,
    /// Coordinates already written as `level_chunk_with_light`.
    sent: BTreeSet<(i32, i32)>,
    /// Chunks written since the current burst began (the burst's log line).
    written: i32,
    /// Sub-batches written since the current burst began.
    batches: u32,
    /// Chunk cache centre the framing counts against.
    center_x: i32,
    /// Chunk cache centre, second coordinate.
    center_z: i32,
    /// View distance the window is built at.
    radius: i32,
    /// Chunks a window move wanted but the ready world does not hold (a
    /// world edge is not a hole): counted, logged, never sent.
    not_cached: u64,
    /// Whether any sub-batch has gone out yet: only the join's opening waits
    /// for the pace with [`FIRST_FLUSH`].
    opened: bool,
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
        let mut queue: VecDeque<(i32, i32)> = wanted
            .iter()
            .copied()
            .filter(|&(chunk_x, chunk_z)| ready.position(chunk_x, chunk_z).is_some())
            .collect();
        let not_cached = (wanted.len() - queue.len()) as u64;
        info!(
            radius,
            center_x,
            center_z,
            in_window = wanted.len(),
            sent = queue.len(),
            not_cached,
            "chunk window selected"
        );
        queue
            .make_contiguous()
            .sort_unstable_by_key(|&(chunk_x, chunk_z)| {
                order_key(chunk_x, chunk_z, center_x, center_z)
            });
        Self {
            chunks: ready,
            queue,
            sent: BTreeSet::new(),
            written: 0,
            batches: 0,
            center_x,
            center_z,
            radius,
            not_cached,
            opened: false,
            stage: Stage::First,
        }
    }

    /// The stage the next turn of the loop has to wait out (a copy, so
    /// the loop can hold `&mut self` while it waits).
    pub(super) fn stage(&self) -> Stage {
        self.stage
    }

    /// The chunks already on the wire (the ШАГ 3 invariant tests).
    #[cfg(test)]
    pub(super) fn sent(&self) -> &BTreeSet<(i32, i32)> {
        &self.sent
    }

    /// Whether nothing is left queued (the ШАГ 3 invariant tests).
    #[cfg(test)]
    pub(super) fn is_idle(&self) -> bool {
        self.queue.is_empty()
    }

    /// How many chunks are still queued (log lines and tests).
    pub(super) fn queued(&self) -> usize {
        self.queue.len()
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

    /// Applies one window move (M4.4 B, ШАГ 3): the centre becomes `center`,
    /// the leaving chunks are forgotten or dropped and the entering ones are
    /// queued.
    ///
    /// Returns the chunks the caller must write as `forget_level_chunk`, in
    /// ascending `(x, z)` — the traced order (ШАГ 1): a leaving chunk still
    /// queued is dropped silently (the client never saw it), one already sent
    /// is returned to be forgotten, and one neither queued nor sent is
    /// ignored. The entering chunks join the back of the queue in the order
    /// the tick gave, each only if the ready world holds it.
    ///
    /// Invariant (M4.4 B): after every move, `sent ∪ queue` holds no chunk
    /// outside `window(center)` — asserted in debug builds.
    pub(super) fn move_window(
        &mut self,
        center: (i32, i32),
        add: &[(i32, i32)],
        remove: &[(i32, i32)],
    ) -> Vec<(i32, i32)> {
        self.center_x = center.0;
        self.center_z = center.1;
        let leaving: BTreeSet<(i32, i32)> = remove.iter().copied().collect();
        let mut forget = Vec::new();
        for &cell in remove {
            if self.queue.contains(&cell) {
                // Still queued: the retain below drops it, no packet.
                continue;
            }
            if self.sent.remove(&cell) {
                forget.push(cell);
            }
        }
        if !leaving.is_empty() {
            self.queue.retain(|cell| !leaving.contains(cell));
        }
        let was_idle = self.queue.is_empty();
        let mut misses = 0_u64;
        for &cell in add {
            if self.sent.contains(&cell) || self.queue.contains(&cell) {
                continue;
            }
            if self.chunks.position(cell.0, cell.1).is_some() {
                self.queue.push_back(cell);
            } else {
                misses += 1;
            }
        }
        if misses > 0 {
            self.not_cached += misses;
            info!(
                misses,
                not_cached = self.not_cached,
                center_x = center.0,
                center_z = center.1,
                "window move: chunks missing from the ready cache"
            );
        }
        if was_idle && !self.queue.is_empty() {
            // A new burst begins: its log line counts these chunks only.
            self.written = 0;
            self.batches = 0;
        }
        if !self.queue.is_empty() {
            // The queued chunks are due now; the batch arm writes them.
            self.stage = Stage::Pace { at: Instant::now() };
        }
        debug_assert!(
            self.within_window(),
            "a window move left sent/queue outside the new window"
        );
        forget
    }

    /// Whether every chunk sent or queued sits inside `window(center)` —
    /// the M4.4 B invariant (debug builds and tests).
    #[cfg(debug_assertions)]
    pub(super) fn within_window(&self) -> bool {
        let wanted: BTreeSet<(i32, i32)> = window(self.center_x, self.center_z, self.radius)
            .into_iter()
            .collect();
        self.sent.iter().all(|cell| wanted.contains(cell))
            && self.queue.iter().all(|cell| wanted.contains(cell))
    }

    /// Writes the next sub-batch: `Ok(true)` when nothing is queued any more
    /// (the loop keeps the sender for the next window move), `Ok(false)`
    /// when more is left and the caller must wait out [`Self::stage`] before
    /// asking again.
    pub(super) async fn send_next<W>(
        &mut self,
        writer: &mut W,
        ids: &PacketIds,
        pace: &ChunkPace,
    ) -> Result<bool, ConnectionError>
    where
        W: AsyncWrite + Unpin,
    {
        if self.queue.is_empty() {
            if !self.opened {
                // An empty world still frames its own (empty) join batch.
                write_sub_batch(writer, ids, &[], self.center_x, self.center_z).await?;
                self.opened = true;
                self.batches = 1;
                info!(chunk_count = 0, sub_batches = 1, "chunk batch sent");
            }
            self.stage = Stage::Parked;
            return Ok(true);
        }
        // The size of this sub-batch: the rate the client confirmed, while
        // the first flush before any answer stays within [`FIRST_FLUSH`] —
        // the opening of the recorded vanilla burst.
        let left = self.queue.len();
        let size = match self.stage {
            Stage::First if !pace.known() => left.min(FIRST_FLUSH),
            _ => pace.sub_batch_size(left).unwrap_or(left),
        };
        let mut group: Vec<&LevelChunkWithLight> = Vec::with_capacity(size);
        let mut sent_coords: Vec<(i32, i32)> = Vec::with_capacity(size);
        for _ in 0..size.min(left) {
            let Some((chunk_x, chunk_z)) = self.queue.pop_front() else {
                break;
            };
            if let Some(index) = self.chunks.position(chunk_x, chunk_z) {
                group.push(&self.chunks.chunks()[index]);
                sent_coords.push((chunk_x, chunk_z));
            }
        }
        self.written += write_sub_batch(writer, ids, &group, self.center_x, self.center_z).await?;
        self.batches += 1;
        self.opened = true;
        for cell in sent_coords {
            self.sent.insert(cell);
        }
        if self.queue.is_empty() {
            info!(
                chunk_count = self.written,
                sub_batches = self.batches.max(1),
                "chunk batch sent"
            );
            self.stage = Stage::Parked;
            return Ok(true);
        }
        let now = Instant::now();
        self.stage = match self.stage {
            // The opening is out; an answer may already have arrived while
            // it was writing.
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

/// Waits out one stage of a burst: nothing for the opening sub-batch, a
/// deadline for the pace answer, a game tick for the paced tail, and
/// forever while nothing is queued ([`Stage::Parked`]).
///
/// `None` (no sender yet) parks forever too — the guarded `select!` arm
/// of the play loop never polls it then.
pub(super) async fn wait_turn(stage: Option<Stage>) -> Option<()> {
    match stage? {
        Stage::First => {}
        Stage::AwaitPace { deadline } => tokio::time::sleep_until(deadline).await,
        Stage::Pace { at } => tokio::time::sleep_until(at).await,
        Stage::Parked => std::future::pending::<()>().await,
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

/// Writes one window move to the wire (M4.4 B, ШАГ 3) and applies it to the
/// sender.
///
/// Order is the traced one (ШАГ 1): `set_chunk_cache_center`(new) first, then
/// every `forget_level_chunk` [`PendingBatch::move_window`] returns — the
/// chunks that were already sent and left the window, ascending `(x, z)` —
/// and the entering chunks are left in the queue for the batch arm to drain
/// as usual. Returns how many chunks are queued after the move.
///
/// Kept as its own function so the play loop's `StreamChunks` arm stays a
/// one-liner and the wire order is testable without a game thread.
pub(super) async fn write_window_move<W>(
    writer: &mut W,
    ids: &PacketIds,
    batch: &mut PendingBatch,
    center: (i32, i32),
    add: &[(i32, i32)],
    remove: &[(i32, i32)],
) -> Result<usize, ConnectionError>
where
    W: AsyncWrite + Unpin,
{
    let center_packet = SetCenterChunk {
        chunk_x: center.0,
        chunk_z: center.1,
    };
    write_tracked(
        writer,
        ProtocolState::Play,
        ids.id(SetCenterChunk::KEY),
        &center_packet.encode_payload(),
    )
    .await?;
    for (chunk_x, chunk_z) in batch.move_window(center, add, remove) {
        let forget_packet = ForgetLevelChunk { chunk_x, chunk_z };
        write_tracked(
            writer,
            ProtocolState::Play,
            ids.id(ForgetLevelChunk::KEY),
            &forget_packet.encode_payload(),
        )
        .await?;
    }
    Ok(batch.queued())
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
