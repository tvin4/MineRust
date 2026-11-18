//! The writer thread behind the tick (Шаг 22b).
//!
//! The game thread never touches the disk: it hands a batch of dirty chunks to
//! a bounded queue with [`WorldSave::submit`] and reads the writer's
//! acknowledgements with [`WorldSave::completed`] once per autosave. The
//! thread itself is a plain named thread (not a tokio task), so the play loop
//! is never blocked by a file write and no lock is held across I/O
//! (`docs/DECISIONS.md` «Шаг 22b»).
//!
//! A periodic full queue is never waited on: the batch is refused (`false`)
//! and the chunks stay dirty for the next pass. After the tick loop stops,
//! [`Runtime::flush`] queues one newest complete snapshot with a deadline and
//! reports actual disk errors; [`Writer::shutdown`] then drains and joins with
//! its own bounded deadline.

use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use net::game::WorldSave;
use net::store::{ChunkSave, ChunkSaved};
use tracing::{debug, warn};

use crate::format::ChunkDiff;
use crate::store::{Store, StoreError};

/// Capacity of the writer queue when the caller does not choose one: 32
/// batches of up to 256 chunks each.
pub const DEFAULT_CAPACITY: usize = 32;

/// One message for the writer thread.
enum Command {
    /// Write these chunks, in this order.
    Save(Vec<ChunkSave>),
    /// Write the newest complete shutdown snapshot and report every error to
    /// the server instead of reducing it to an unobserved acknowledgement.
    Flush(Vec<ChunkSave>, mpsc::Sender<Result<(), StoreError>>),
    /// Drain everything queued before it and stop.
    Shutdown,
}

/// The game thread's handle: the bounded batch queue and the reply channel.
pub struct Runtime {
    sender: SyncSender<Command>,
    /// Reports drained by [`WorldSave::completed`]. A `Mutex` only because
    /// `Receiver` is not `Sync`; the lock is taken once per autosave pass and
    /// never held while a chunk is written.
    acks: Mutex<Receiver<ChunkSaved>>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("worldsave::Runtime").finish_non_exhaustive()
    }
}

impl WorldSave for Runtime {
    fn submit(&self, chunks: Vec<ChunkSave>) -> bool {
        if chunks.is_empty() {
            return true;
        }
        self.sender.try_send(Command::Save(chunks)).is_ok()
    }

    fn completed(&self) -> Vec<ChunkSaved> {
        let acks = self
            .acks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        acks.try_iter().collect()
    }
}

impl Runtime {
    /// Writes the final complete dirty snapshot after the game thread has
    /// stopped and waits for the result within `timeout`.
    ///
    /// The command is ordered after every periodic batch already in the
    /// bounded queue, so its files are the last version on disk. Unlike
    /// periodic saves, an I/O error is returned to the server and cannot be
    /// logged as a successful flush.
    pub fn flush(&self, chunks: Vec<ChunkSave>, timeout: Duration) -> Result<(), StoreError> {
        if chunks.is_empty() {
            return Ok(());
        }
        let deadline = Instant::now() + timeout;
        let (done, wait) = mpsc::channel();
        send_until(
            &self.sender,
            Command::Flush(chunks, done),
            deadline,
            "queue the final flush",
        )?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        match wait.recv_timeout(remaining) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err(StoreError::WriterTimeout {
                action: "finish the final flush",
            }),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(StoreError::WriterGone),
        }
    }
}

/// The writer thread and its shutdown handle.
pub struct Writer {
    sender: SyncSender<Command>,
    handle: Option<JoinHandle<()>>,
}

impl Writer {
    /// Stops the writer, draining every queued batch first.
    ///
    /// The stop message is queued behind the pending batches; the thread is
    /// joined within `timeout`. A timeout is an error, not a panic: the
    /// thread is detached and the server keeps shutting down.
    pub fn shutdown(mut self, timeout: Duration) -> Result<(), StoreError> {
        let deadline = Instant::now() + timeout;
        send_until(&self.sender, Command::Shutdown, deadline, "queue shutdown")?;
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        let (done, wait) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(handle.join());
        });
        let remaining = deadline.saturating_duration_since(Instant::now());
        match wait.recv_timeout(remaining) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(StoreError::WriterGone),
            Err(_) => Err(StoreError::ShutdownTimeout),
        }
    }
}

/// Sends one control command without letting a full bounded queue turn a
/// nominal timeout into an unbounded `send` wait.
fn send_until(
    sender: &SyncSender<Command>,
    mut command: Command,
    deadline: Instant,
    action: &'static str,
) -> Result<(), StoreError> {
    loop {
        match sender.try_send(command) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Full(returned)) => {
                if Instant::now() >= deadline {
                    return Err(StoreError::WriterTimeout { action });
                }
                command = returned;
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(TrySendError::Disconnected(_)) => return Err(StoreError::WriterGone),
        }
    }
}

/// Starts the writer thread for `store`.
///
/// `capacity` is clamped to at least one and bounds how many batches may be
/// pending; [`DEFAULT_CAPACITY`] is a sane value.
pub fn spawn(store: Store, capacity: usize) -> Result<(Runtime, Writer), StoreError> {
    let (sender, receiver) = mpsc::sync_channel::<Command>(capacity.max(1));
    let (ack_sender, ack_receiver) = mpsc::channel::<ChunkSaved>();
    let handle = std::thread::Builder::new()
        .name("worldsave-writer".to_owned())
        .spawn(move || run(store, receiver, ack_sender))
        .map_err(|source| StoreError::Spawn { source })?;
    let runtime = Runtime {
        sender: sender.clone(),
        acks: Mutex::new(ack_receiver),
    };
    let writer = Writer {
        sender,
        handle: Some(handle),
    };
    Ok((runtime, writer))
}

/// Consumes the queue until the stop message or the channel closes.
fn run(store: Store, receiver: Receiver<Command>, acks: mpsc::Sender<ChunkSaved>) {
    while let Ok(command) = receiver.recv() {
        match command {
            Command::Save(batch) => {
                for save in batch {
                    let chunk_x = save.chunk_x;
                    let chunk_z = save.chunk_z;
                    let generation = save.generation;
                    let ok = match write_chunk(&store, save) {
                        Ok(()) => true,
                        Err(err) => {
                            // A failed periodic write never brings the server
                            // down: the negative ack makes the chunk eligible
                            // for the next pass.
                            warn!(%err, "could not write a chunk diff");
                            false
                        }
                    };
                    let _ = acks.send(ChunkSaved {
                        chunk_x,
                        chunk_z,
                        generation,
                        ok,
                    });
                }
            }
            Command::Flush(batch, done) => {
                let mut result = Ok(());
                for save in batch {
                    if let Err(err) = write_chunk(&store, save) {
                        if result.is_ok() {
                            result = Err(err);
                        }
                    }
                }
                let _ = done.send(result);
            }
            Command::Shutdown => break,
        }
    }
    debug!("world-save writer stopped");
}

/// Writes one queued chunk as the diff document the store owns.
fn write_chunk(store: &Store, save: ChunkSave) -> Result<(), StoreError> {
    store.write_diff(&ChunkDiff {
        chunk_x: save.chunk_x,
        chunk_z: save.chunk_z,
        changes: save.changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::WorldIdentity;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "minerust-worldsave-writer-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn store(dir: &std::path::Path) -> Store {
        Store::open(
            dir,
            &WorldIdentity {
                template: "E:/worlds/template".to_owned(),
                level_dat_sha256: "abc".to_owned(),
                data_version: 5023,
            },
            false,
        )
        .expect("store")
    }

    fn save(generation: u64, changes: Vec<(u32, i32)>) -> ChunkSave {
        ChunkSave {
            chunk_x: 0,
            chunk_z: 0,
            generation,
            changes,
        }
    }

    /// A submitted batch reaches the disk, its acknowledgement carries the
    /// generation, and shutdown drains what was still queued.
    #[test]
    fn a_batch_is_written_and_acknowledged() {
        let dir = temp_dir("drain");
        let store = store(&dir);
        let reader = store.clone();
        let (runtime, writer) = spawn(store, DEFAULT_CAPACITY).expect("spawn");
        assert!(runtime.submit(vec![save(3, vec![(4, 1)])]));
        writer
            .shutdown(Duration::from_secs(10))
            .expect("writer stops");

        let diff = reader.read_diff(0, 0).expect("read").expect("file");
        assert_eq!(diff.changes, vec![(4, 1)]);
        let acks = runtime.completed();
        assert_eq!(acks.len(), 1);
        assert_eq!(acks[0].generation, 3);
        assert!(acks[0].ok);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A batch that returns its chunk to the template deletes the file and is
    /// still acknowledged, so the store may forget the chunk.
    #[test]
    fn an_empty_batch_removes_the_file() {
        let dir = temp_dir("empty");
        let store = store(&dir);
        let reader = store.clone();
        let (runtime, writer) = spawn(store, DEFAULT_CAPACITY).expect("spawn");
        assert!(runtime.submit(vec![save(1, vec![(4, 1)])]));
        assert!(runtime.submit(vec![save(2, Vec::new())]));
        writer
            .shutdown(Duration::from_secs(10))
            .expect("writer stops");
        assert!(reader.read_diff(0, 0).expect("read").is_none());
        let acks = runtime.completed();
        assert_eq!(acks.len(), 2);
        assert_eq!(acks[1].generation, 2);
        assert!(acks[1].ok);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Submitting nothing is a success, not a queue message.
    #[test]
    fn an_empty_submit_is_accepted_without_a_message() {
        let dir = temp_dir("nothing");
        let (runtime, writer) = spawn(store(&dir), DEFAULT_CAPACITY).expect("spawn");
        assert!(runtime.submit(Vec::new()));
        assert!(runtime.completed().is_empty());
        writer
            .shutdown(Duration::from_secs(10))
            .expect("writer stops");
        let _ = fs::remove_dir_all(&dir);
    }

    /// The final flush reports completion only after its file is durable.
    #[test]
    fn a_final_flush_writes_its_complete_batch() {
        let dir = temp_dir("flush");
        let store = store(&dir);
        let reader = store.clone();
        let (runtime, writer) = spawn(store, DEFAULT_CAPACITY).expect("spawn");
        runtime
            .flush(vec![save(4, vec![(7, 1)])], Duration::from_secs(10))
            .expect("flush");
        assert_eq!(
            reader.read_diff(0, 0).expect("read").expect("file").changes,
            vec![(7, 1)]
        );
        writer
            .shutdown(Duration::from_secs(10))
            .expect("writer stops");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A full bounded queue reaches the flush deadline instead of blocking
    /// forever before the advertised timeout begins.
    #[test]
    fn a_final_flush_times_out_while_the_queue_stays_full() {
        let (sender, receiver) = mpsc::sync_channel(1);
        sender
            .send(Command::Save(vec![save(1, vec![(4, 1)])]))
            .expect("fill queue");
        let (_ack_sender, ack_receiver) = mpsc::channel();
        let runtime = Runtime {
            sender,
            acks: Mutex::new(ack_receiver),
        };
        assert!(matches!(
            runtime.flush(vec![save(2, vec![(5, 1)])], Duration::from_millis(10)),
            Err(StoreError::WriterTimeout {
                action: "queue the final flush"
            })
        ));
        drop(receiver);
    }

    /// A final disk error is returned to the server; it is not reduced to an
    /// acknowledgement nobody will read after the game thread has stopped.
    #[test]
    fn a_final_flush_reports_a_disk_error() {
        let dir = temp_dir("flush-error");
        let store = store(&dir);
        fs::remove_dir_all(dir.join(crate::CHUNKS_DIR)).expect("remove chunks dir");
        fs::write(dir.join(crate::CHUNKS_DIR), b"not a directory").expect("block path");
        let (runtime, writer) = spawn(store, DEFAULT_CAPACITY).expect("spawn");
        assert!(matches!(
            runtime.flush(vec![save(4, vec![(7, 1)])], Duration::from_secs(10)),
            Err(StoreError::Io { .. })
        ));
        writer
            .shutdown(Duration::from_secs(10))
            .expect("writer stops");
        let _ = fs::remove_dir_all(&dir);
    }
}
