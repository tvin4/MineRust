//! The chunk loader: world reads off the connection tasks.
//!
//! Chunks are read from disk and encoded on a pool of dedicated worker
//! threads (step 3.3c): a connection task only queues a job over a bounded
//! channel and awaits the reply, so no file I/O ever runs inside the play
//! loop and the periodic keep-alive keeps its cadence while the world is
//! being read.
//!
//! The shape is deliberately lock-free: every worker owns a bounded queue
//! of its own (round-robin over the workers), each job carries its reply
//! channel, and the only shared state is the queue list itself. No `Mutex`
//! stands between a connection and a chunk, in the hot tick or anywhere
//! else (docs/DECISIONS.md, step 3.3).
//!
//! SOURCE: measured behaviour of tokio 1.54 — a `spawn_blocking` task in
//! progress suspends the auto-advance of a paused test clock indefinitely
//! (verified by the tests of this module), which would leave the
//! keep-alive test of `connection.rs` waiting forever. The workers are
//! plain named threads for that reason; the runtime never tracks them.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};
use tracing::debug;

use crate::chunks::ReadyChunks;

/// What a pool read fails with; sources box their own error type.
pub type LoadError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// The world behind a [`Loader`]: reads happen on the blocking pool, one
/// call per batch, never on a connection task.
pub trait ChunkSource: Send + Sync + 'static {
    /// The ready world of this source right now, behind a shared `Arc`:
    /// the batch of a template world is built once and every join receives
    /// the same immutable value, so a connection only ever clones the
    /// pointer (part A 2, the cache), not the payloads. The value carries
    /// the `(x, z) -> position` index a join filters its window with
    /// (M4.4 A). A failing read reports the data problem instead of a
    /// default.
    fn load_batch(&self) -> Result<Arc<ReadyChunks>, LoadError>;
}

/// The whole loader pool has gone away (shut down or panicked earlier).
#[derive(Debug, thiserror::Error)]
#[error("the chunk loader pool is shut down")]
pub struct LoaderShutDown;

/// One unit of world work: the reply channel a batch arrives on.
struct Job {
    /// Where the built batch (or its error) is delivered.
    reply: oneshot::Sender<Result<Arc<ReadyChunks>, LoadError>>,
}

/// Blocking workers in the pool (spawn_blocking threads, one queue each).
const WORKERS: usize = 2;

/// Jobs one worker's queue holds before the sender waits (backpressure).
const QUEUE: usize = 4;

/// A handle to the loader pool: cheap to clone, one clone per connection.
///
/// Cloning shares the worker queues; [`Self::load_batch`] spreads jobs over
/// them round-robin so one slow read cannot starve another connection.
#[derive(Clone)]
pub struct Loader {
    /// The bounded queue of every worker of the pool.
    inner: Arc<Inner>,
}

/// The shared queues and the round-robin cursor.
struct Inner {
    /// One bounded sender per worker; never empty.
    queues: Vec<mpsc::Sender<Job>>,
    /// Which queue takes the next job.
    next: AtomicUsize,
}

impl Loader {
    /// Spawns the pool for `source`: `WORKERS` named threads, each fed
    /// through its own bounded queue. The workers live as long as the
    /// source and their queues do (they exit when every handle is gone).
    ///
    /// This is called at server start, not per request: pool construction
    /// is the only place a thread is touched, and connections only clone
    /// and send. The threads are `std` threads, never `spawn_blocking` —
    /// see the reason in this module's doc comment.
    pub fn spawn(source: Arc<dyn ChunkSource>) -> std::io::Result<Self> {
        let mut queues = Vec::with_capacity(WORKERS);
        for index in 0..WORKERS {
            let (tx, mut rx) = mpsc::channel::<Job>(QUEUE);
            queues.push(tx);
            let source = Arc::clone(&source);
            std::thread::Builder::new()
                .name(format!("chunk-loader-{index}"))
                .spawn(move || {
                    // The worker parks on its own queue here: this thread
                    // is the pool, never a connection task.
                    while let Some(job) = rx.blocking_recv() {
                        let _ = job.reply.send(source.load_batch());
                    }
                    debug!("chunk loader worker finished");
                })?;
        }
        Ok(Self {
            inner: Arc::new(Inner {
                queues,
                next: AtomicUsize::new(0),
            }),
        })
    }

    /// Queues one batch read and returns the channel its reply arrives on.
    ///
    /// The `await` is the bounded queue itself: when every worker is busy
    /// the caller queues instead of reading the disk itself, which is the
    /// backpressure the pool exists for.
    pub async fn load_batch(
        &self,
    ) -> Result<oneshot::Receiver<Result<Arc<ReadyChunks>, LoadError>>, LoaderShutDown> {
        let index = self.inner.next.fetch_add(1, Ordering::Relaxed) % self.inner.queues.len();
        let (reply, received) = oneshot::channel();
        self.inner.queues[index]
            .send(Job { reply })
            .await
            .map_err(|_| LoaderShutDown)?;
        Ok(received)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use protocol::{
        ChunkSection, HeightmapRecord, LevelChunkWithLight, LightData, PaletteContainer,
    };

    /// A source that answers from the thread it runs on and records that
    /// thread, so a test can prove the read left the runtime.
    struct EchoSource {
        /// How many payloads the batch carries.
        count: usize,
        /// Thread name of the worker that called [`ChunkSource::load_batch`].
        seen: std::sync::Mutex<Vec<String>>,
    }

    impl ChunkSource for EchoSource {
        fn load_batch(&self) -> Result<Arc<ReadyChunks>, LoadError> {
            let thread = std::thread::current();
            let name = thread.name().unwrap_or("<unnamed>").to_owned();
            self.seen.lock().expect("test lock uncontended").push(name);
            Ok(Arc::new(ReadyChunks::new(
                (0..self.count).map(test_chunk).collect(),
            )))
        }
    }

    /// One well-formed payload: one section, one heightmap, a light block
    /// whose masks match an empty array set.
    fn test_chunk(index: usize) -> LevelChunkWithLight {
        LevelChunkWithLight {
            chunk_x: index as i32,
            chunk_z: -2,
            heightmaps: vec![HeightmapRecord {
                id: 4,
                words: vec![0],
            }],
            sections: vec![ChunkSection {
                non_air: 0,
                fluid: 0,
                blocks: PaletteContainer::Single(0),
                biomes: PaletteContainer::Single(1),
            }],
            block_entities: Vec::new(),
            light: LightData {
                sky_mask: vec![0x00],
                block_mask: vec![0x00],
                empty_sky_mask: vec![0x01],
                empty_block_mask: vec![0x01],
                sky_arrays: Vec::new(),
                block_arrays: Vec::new(),
            },
        }
    }

    /// Waits on a real thread, so a stalled clock reports itself instead of
    /// parking the test forever: the 4 s bound is real time.
    fn real_deadline() -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(4));
            let _ = tx.send(());
        });
        rx
    }

    /// The paused clock jumps to the next timer by itself: the test only
    /// takes a moment of real time even though the sleep is an hour long.
    #[tokio::test]
    async fn a_paused_clock_advances_on_its_own() {
        tokio::time::pause();
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(3600)) => {}
            _ = real_deadline() => panic!("stalled: the clock never advanced"),
        }
    }

    /// The measured tokio behaviour the pool's design rests on: a
    /// `spawn_blocking` task in progress freezes a paused clock (the
    /// timer never fires — neither in virtual nor in real time), so a
    /// loader built on `spawn_blocking` would make the keep-alive test of
    /// `connection.rs` wait forever. The pool therefore uses plain
    /// threads, which do not have this effect (the test above passes).
    ///
    /// Ignored because it waits out a real four-second bound; run it with
    /// `--ignored` when touching the loader's threading.
    #[tokio::test]
    #[ignore = "a real-time four-second bound keeps the normal run fast"]
    async fn a_spawn_blocking_task_freezes_the_paused_clock() {
        tokio::time::pause();
        let (release, held) = std::sync::mpsc::channel::<()>();
        tokio::task::spawn_blocking(move || {
            // Kept in progress until the test drops the sender, so the
            // whole select runs with the blocking task outstanding.
            let _ = held.recv_timeout(Duration::from_secs(60));
        });
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(3600)) => {
                panic!("tokio changed: spawn_blocking no longer freezes the clock")
            }
            // The real-time deadline fired first: the paused clock stood
            // still for as long as the blocking task was in progress.
            _ = real_deadline() => {}
        }
        drop(release); // the worker leaves
    }

    /// The reply of a job is the source's batch, and the source ran on one of
    /// the pool's named worker threads, not inside the awaiting task.
    #[tokio::test]
    async fn a_batch_arrives_from_a_pool_worker() {
        let source = Arc::new(EchoSource {
            count: 3,
            seen: std::sync::Mutex::new(Vec::new()),
        });
        let loader =
            Loader::spawn(Arc::clone(&source) as Arc<dyn ChunkSource>).expect("worker threads");

        let received = loader.load_batch().await.expect("pool alive");
        let chunks = received.await.expect("reply").expect("read ok");
        assert_eq!(chunks.len(), 3, "the source's whole batch");
        assert_eq!(chunks.chunks()[0].chunk_x, 0);

        let seen = source.seen.lock().expect("test lock uncontended");
        assert_eq!(seen.len(), 1, "one worker picked the job up");
        assert!(
            seen[0].starts_with("chunk-loader-"),
            "the reading thread is the pool's {}, not this test's",
            seen[0]
        );
    }
}
