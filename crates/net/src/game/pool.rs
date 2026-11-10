//! The region worker pool of the game thread (ARCH-M4 §1, M4.3 A2).
//!
//! `K` `std::thread` workers own no state between ticks: at the start of a
//! tick the scheduler **moves** every region to a worker over an
//! `mpsc::channel`, the worker ticks it and sends it back, and the scheduler
//! merges the results in `region_id` order once all of them are in. There is
//! no shared mutable state and no `Mutex`/`RwLock` on this path; ownership
//! itself is the lock.
//!
//! The pool is a mechanism only in M4.3: one region means the scheduler
//! keeps ticking inline (see [`super::scheduler::Scheduler`]), so the live
//! server behaves exactly as in M4.2.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SendError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tracing::{error, warn};

use super::grid::RegionGrid;
use super::region::{Region, TickOutcome};

/// How long the scheduler waits for one region's result before giving up on
/// the pool for this tick (a worker that panicked would never answer).
const RESULT_TIMEOUT: Duration = Duration::from_secs(2);

/// How long the pool waits for its workers to leave on shutdown.
const STOP_TIMEOUT: Duration = Duration::from_secs(2);

/// One region handed to a worker for exactly one tick.
#[derive(Debug)]
pub(crate) struct Job {
    /// The region, owned by the worker until it sends it back.
    pub region: Region,
    /// Tick number the region is ticked for.
    pub tick: u64,
    /// The tick's `now`, shared by every region of the tick.
    pub now: Instant,
    /// The chunk → region grid the region needs to detect transfers.
    pub grid: RegionGrid,
}

/// A ticked region on its way back to the scheduler.
#[derive(Debug)]
pub(crate) struct Done {
    /// The same region, to be put back into the scheduler.
    pub region: Region,
    /// What the region produced while ticking.
    pub outcome: TickOutcome,
}

/// `K` long-lived worker threads and the channels around them.
#[derive(Debug)]
pub(crate) struct Pool {
    /// One job channel per worker (round-robin dispatch).
    jobs: Vec<Sender<Job>>,
    /// Every worker sends its result here.
    results: Receiver<Done>,
    /// Workers announce their exit here (one message each).
    done: Receiver<()>,
    /// The worker threads, joined with a timeout on shutdown.
    workers: Vec<JoinHandle<()>>,
}

impl Pool {
    /// Starts `workers` (`>= 1`) threads. Fails only when a thread cannot be
    /// spawned; the caller then keeps ticking inline.
    pub(crate) fn new(workers: usize) -> std::io::Result<Self> {
        let (results, results_rx) = mpsc::channel();
        let (done, done_rx) = mpsc::channel();
        let mut jobs = Vec::with_capacity(workers);
        let mut handles = Vec::with_capacity(workers);
        for index in 0..workers {
            let (job_tx, job_rx) = mpsc::channel::<Job>();
            let results = results.clone();
            let done = done.clone();
            let handle = std::thread::Builder::new()
                .name(format!("minecraft-region-{index}"))
                .spawn(move || worker(job_rx, results, done))?;
            jobs.push(job_tx);
            handles.push(handle);
        }
        // The pool itself keeps no sender, so dropping the pool closes every
        // job channel and the workers leave instead of waiting forever.
        drop(results);
        drop(done);
        Ok(Self {
            jobs,
            results: results_rx,
            done: done_rx,
            workers: handles,
        })
    }

    /// Number of workers (always positive for an existing pool).
    pub(crate) fn workers(&self) -> usize {
        self.jobs.len()
    }

    /// Sends one region to worker `slot % K`.
    ///
    /// A full/closed channel returns the job back so the caller can tick the
    /// region inline rather than lose it.
    pub(crate) fn send(&self, slot: usize, job: Job) -> Result<(), SendError<Job>> {
        let Some(sender) = self.jobs.get(slot % self.workers().max(1)) else {
            return Err(SendError(job));
        };
        sender.send(job)
    }

    /// Waits for the next ticked region (the barrier is the scheduler
    /// calling this exactly as many times as it dispatched).
    pub(crate) fn collect(&self) -> Option<Done> {
        match self.results.recv_timeout(RESULT_TIMEOUT) {
            Ok(done) => Some(done),
            Err(RecvTimeoutError::Timeout) => {
                error!(
                    timeout_ms = RESULT_TIMEOUT.as_millis(),
                    "a region worker did not return its region"
                );
                None
            }
            Err(RecvTimeoutError::Disconnected) => {
                error!("every region worker is gone");
                None
            }
        }
    }
}

impl Drop for Pool {
    /// Closes the job channels and waits at most [`STOP_TIMEOUT`] for the
    /// workers to leave; a worker still running after that is detached (the
    /// process is exiting anyway) and the fact is logged.
    fn drop(&mut self) {
        self.jobs.clear();
        let deadline = Instant::now() + STOP_TIMEOUT;
        let mut finished = 0usize;
        while finished < self.workers.len() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match self.done.recv_timeout(left) {
                Ok(()) => finished += 1,
                Err(_) => break,
            }
        }
        if finished < self.workers.len() {
            warn!(
                workers = self.workers.len(),
                finished, "some region workers did not stop in time; detaching them"
            );
        }
        for handle in self.workers.drain(..finished) {
            if handle.join().is_err() {
                warn!("a region worker panicked");
            }
        }
    }
}

/// One worker: tick every region it is handed and send it straight back.
fn worker(jobs: Receiver<Job>, results: Sender<Done>, done: Sender<()>) {
    while let Ok(mut job) = jobs.recv() {
        let outcome = job.region.tick(job.tick, job.now, job.grid);
        if results
            .send(Done {
                region: job.region,
                outcome,
            })
            .is_err()
        {
            break;
        }
    }
    let _ = done.send(());
}
