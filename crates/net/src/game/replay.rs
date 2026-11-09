//! The JSONL replay journal of the game thread (ARCH-M4 §1).
//!
//! One line per event, written the moment the tick **applies** it — not when
//! the socket reads it. That is what takes the network out of the equation:
//! a replay is the same journal applied to the same world state.
//!
//! The field set is M4.1 **[hypothesis]**: no vanilla trace backs it (see
//! `docs/ARCH-M4.md` §1). Events outside a region carry `region: null`; the
//! `kind` names the applied message and `payload` its fields.

use std::io::{BufWriter, Write};

use serde::Serialize;
use tracing::warn;

use super::messages::PlayerId;
use super::region::RegionId;

/// One applied event, exactly the fields of the ARCH-M4 §1 line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReplayRecord<'a> {
    /// Tick in which the event was applied.
    pub tick: u64,
    /// Order of the event inside the tick.
    pub seq: u32,
    /// Region that applied it; `None` for events outside a region.
    pub region: Option<RegionId>,
    /// Connection the event came from.
    pub src: PlayerId,
    /// Event kind: `join`, `teleport_ack`, `quit` (movement joins in M4.2).
    pub kind: &'a str,
    /// Event fields, kept as a JSON object so the format can grow.
    pub payload: serde_json::Value,
}

/// The replay journal of one run.
///
/// A journal with no sink is disabled: [`Replay::record`] does nothing and
/// costs one branch. The writer is generic so a test can capture the lines
/// into a `Vec<u8>` and the server writes them to a `File`.
#[derive(Debug)]
pub struct Replay<W: Write> {
    sink: Option<BufWriter<W>>,
    seq: u32,
}

impl<W: Write> Replay<W> {
    /// A journal that writes to `sink`, or a disabled one when `None`.
    pub fn new(sink: Option<W>) -> Self {
        Self {
            sink: sink.map(BufWriter::new),
            seq: 0,
        }
    }

    /// Whether the journal will record anything.
    pub fn enabled(&self) -> bool {
        self.sink.is_some()
    }

    /// Resets the per-tick order counter; the tick calls this once.
    pub fn start_tick(&mut self) {
        self.seq = 0;
    }

    /// Records one applied event with the next order number of this tick.
    ///
    /// A write failure is logged and never fatal: a debug journal must not
    /// stop the tick.
    pub fn record(
        &mut self,
        tick: u64,
        region: Option<RegionId>,
        src: PlayerId,
        kind: &'static str,
        payload: serde_json::Value,
    ) {
        let Some(sink) = self.sink.as_mut() else {
            return;
        };
        let record = ReplayRecord {
            tick,
            seq: self.seq,
            region,
            src,
            kind,
            payload,
        };
        self.seq += 1;
        if let Err(err) = write_line(sink, &record) {
            warn!(%err, "could not write the replay line");
        }
    }

    /// Flushes the journal to its sink.
    pub fn flush(&mut self) -> std::io::Result<()> {
        match self.sink.as_mut() {
            Some(sink) => sink.flush(),
            None => Ok(()),
        }
    }

    /// Gives the sink back (flushing first), for a test that reads it.
    pub fn into_sink(self) -> Option<W> {
        self.sink.and_then(|sink| match sink.into_inner() {
            Ok(writer) => Some(writer),
            Err(err) => {
                warn!(error = %err.error(), "could not flush the replay journal");
                None
            }
        })
    }
}

/// Writes one record as a JSON line and flushes it, so the line is on disk
/// the moment the tick applied the event.
fn write_line<W: Write>(sink: &mut BufWriter<W>, record: &ReplayRecord<'_>) -> std::io::Result<()> {
    serde_json::to_writer(&mut *sink, record).map_err(std::io::Error::from)?;
    sink.write_all(b"\n")?;
    sink.flush()
}
