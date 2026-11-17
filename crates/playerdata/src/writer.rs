//! The background writer: the tick thread snapshots players and hands the
//! snapshot to a dedicated I/O thread.
//!
//! Writing a player file is disk work (gzip, sync, rename) that must never
//! run on the tick thread; the tick only builds a [`Snapshot`] and sends it.
//! One thread consumes the queue in order, so per player the writes land in
//! the order they were made: the queue is FIFO and a snapshot the tick sends
//! is never reordered past an earlier one.
//!
//! The queue is bounded and a send never blocks: a full queue makes
//! [`Sink::snapshot`] return `false` and the caller logs the dropped save
//! (the next autosave writes the newer state anyway). At server stop the
//! queue is drained first — [`Writer::shutdown`] queues the stop behind every
//! pending save.

use std::sync::mpsc::{self, Receiver, SyncSender};
use std::thread::JoinHandle;
use std::time::Duration;

use tracing::{debug, warn};

use crate::error::PlayerdataError;
use crate::state::Snapshot;
use crate::store::Store;

/// Capacity of the writer queue when the caller does not choose one.
pub const DEFAULT_CAPACITY: usize = 256;

/// One message for the writer thread.
enum Command {
    /// Write this snapshot.
    Save(Snapshot),
    /// Drain everything queued before it and stop.
    Shutdown,
}

/// Handle the game thread keeps to hand snapshots to the writer.
///
/// Cheap to clone (one clone per region/player is fine): every clone feeds
/// the same single writer thread, which is what keeps a player's saves in
/// order.
#[derive(Debug, Clone)]
pub struct Sink {
    sender: SyncSender<Command>,
}

impl Sink {
    /// Queues one snapshot for writing.
    ///
    /// Never blocks: `false` means the queue was full or the writer is gone,
    /// and the caller logs the dropped save.
    pub fn snapshot(&self, snapshot: Snapshot) -> bool {
        self.sender.try_send(Command::Save(snapshot)).is_ok()
    }
}

/// The writer thread and its shutdown handle.
pub struct Writer {
    sender: SyncSender<Command>,
    handle: Option<JoinHandle<()>>,
}

impl Writer {
    /// Stops the writer, draining every queued save first.
    ///
    /// The stop message is queued behind the pending saves; the thread is
    /// joined within `timeout`. A timeout is an error, not a panic: the
    /// thread is detached and the server keeps shutting down.
    pub fn shutdown(mut self, timeout: Duration) -> Result<(), PlayerdataError> {
        // Not on the tick thread, so a full queue may block briefly here.
        let _ = self.sender.send(Command::Shutdown);
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        // `JoinHandle` has no timed join; park a helper on it instead.
        let (done, wait) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = done.send(handle.join());
        });
        match wait.recv_timeout(timeout) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(_)) => Err(PlayerdataError::WriterGone),
            Err(_) => Err(PlayerdataError::ShutdownTimeout),
        }
    }
}

/// Starts the writer thread for `store`.
///
/// `capacity` is clamped to at least one and bounds how many snapshots may be
/// pending; `DEFAULT_CAPACITY` is a sane value.
pub fn spawn(store: Store, capacity: usize) -> Result<(Sink, Writer), PlayerdataError> {
    let (sender, receiver) = mpsc::sync_channel::<Command>(capacity.max(1));
    let handle = std::thread::Builder::new()
        .name("playerdata-writer".to_owned())
        .spawn(move || run(store, receiver))
        .map_err(|source| PlayerdataError::Spawn { source })?;
    let writer = Writer {
        sender: sender.clone(),
        handle: Some(handle),
    };
    Ok((Sink { sender }, writer))
}

/// Consumes the queue until the stop message or the channel closes.
fn run(store: Store, receiver: Receiver<Command>) {
    while let Ok(command) = receiver.recv() {
        match command {
            Command::Save(snapshot) => {
                if let Err(err) = store.write(&snapshot) {
                    // A failed write never brings the server down.
                    warn!(%err, "could not write a player snapshot");
                }
            }
            Command::Shutdown => break,
        }
    }
    debug!("playerdata writer stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Pose;
    use std::fs;
    use std::path::PathBuf;

    /// A unique directory under the system temp root.
    fn temp_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "minerust-playerdata-writer-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn uuid(byte: u8) -> protocol::Uuid {
        [byte; 16]
    }

    fn snapshot(uuid: protocol::Uuid) -> Snapshot {
        Snapshot {
            uuid,
            pose: Pose {
                x: 1.0,
                y: 64.0,
                z: 2.0,
                yaw: 90.0,
                pitch: 0.0,
            },
            on_ground: true,
            health: 20.0,
            food: 20,
            saturation: 5.0,
            game_mode: 0,
        }
    }

    #[test]
    fn a_snapshot_is_written_and_shutdown_drains_the_queue() {
        let dir = temp_dir("drain");
        let store = Store::open(&dir, 5023, &[]).expect("store");
        let reader = store.clone();
        let (sink, writer) = spawn(store, DEFAULT_CAPACITY).expect("spawn");

        assert!(sink.snapshot(snapshot(uuid(0x11))));
        assert!(sink.snapshot(snapshot(uuid(0x22))));
        writer
            .shutdown(Duration::from_secs(10))
            .expect("the writer stops and drains");

        assert_eq!(
            reader.read(uuid(0x11)).expect("read").expect("file").food,
            20
        );
        assert_eq!(
            reader.read(uuid(0x22)).expect("read").expect("file").food,
            20
        );
    }

    #[test]
    fn a_full_queue_is_reported_instead_of_blocking() {
        let dir = temp_dir("full");
        let store = Store::open(&dir, 5023, &[]).expect("store");
        // Capacity one, and the writer is not stopped: with a bounded queue
        // a producer that outruns the writer must be told `false`, never
        // block the tick thread.
        let (sink, writer) = spawn(store, 1).expect("spawn");
        let mut rejected = false;
        for _ in 0..10_000 {
            if !sink.snapshot(snapshot(uuid(0x33))) {
                rejected = true;
                break;
            }
        }
        assert!(rejected, "a bounded queue must reject instead of blocking");
        writer.shutdown(Duration::from_secs(10)).expect("stop");
    }

    #[test]
    fn a_send_after_shutdown_is_refused() {
        let dir = temp_dir("after-stop");
        let store = Store::open(&dir, 5023, &[]).expect("store");
        let (sink, writer) = spawn(store, DEFAULT_CAPACITY).expect("spawn");
        writer.shutdown(Duration::from_secs(10)).expect("stop");
        assert!(!sink.snapshot(snapshot(uuid(0x44))), "the writer is gone");
    }

    #[test]
    fn a_write_error_does_not_stop_the_writer() {
        let dir = temp_dir("error");
        let store = Store::open(&dir, 5023, &[]).expect("store");
        let reader = store.clone();
        let (sink, writer) = spawn(store, DEFAULT_CAPACITY).expect("spawn");

        // A snapshot the store refuses (health out of range) is a write
        // error; the next, valid one still lands.
        let mut broken = snapshot(uuid(0x55));
        broken.health = 21.0;
        assert!(sink.snapshot(broken), "queued; the store rejects it later");
        assert!(sink.snapshot(snapshot(uuid(0x55))));
        writer.shutdown(Duration::from_secs(10)).expect("stop");
        assert_eq!(
            reader.read(uuid(0x55)).expect("read").expect("file").food,
            20
        );
    }
}
