//! ЧАСТЬ III: the entry-speed facts, read back from a walk trace.
//!
//! The bot only *waits* here ([`WalkMode::Preload`]); every number the owner
//! asked for is taken from the JSONL the bot wrote, so vanilla and our own
//! server are measured by exactly the same reader and by the bot's own
//! monotonic clock (`t_ms`, elapsed since the connection attempt).
//!
//! Reported per run:
//!
//! * `intention` → `login` (play) → first `level_chunk_with_light` → last
//!   `chunk_batch_finished` → the N-th `level_chunk_with_light`;
//! * chunk / batch counts, chunk payload bytes and batch sizes;
//! * the gaps between consecutive `chunk_batch_finished` packets.
//!
//! Nothing here concludes anything: min/median/max over the repetitions
//! belong to the report, together with the hypotheses the owner lists.

use std::path::Path;

use serde::Deserialize;

/// Target of the wait as the owner set it: the chunks a `view-distance=10`
/// server is expected to send, `(2 * 10 + 1)^2`.
pub const VIEW_DISTANCE_CHUNKS: u64 = 441;

/// How long the bot waits before giving up on that target.
pub const PRELOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Errors of the trace reader.
#[derive(Debug, thiserror::Error)]
pub enum PreloadError {
    /// The trace file itself could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// Trace path involved.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A line did not have the JSONL shape [`crate::trace`] writes.
    #[error("{path}: line {line} is not the walk JSONL: {source}")]
    Json {
        /// Trace path involved.
        path: String,
        /// 1-based line number of the offending row.
        line: usize,
        /// Underlying parse error.
        #[source]
        source: serde_json::Error,
    },
}

/// One trace row, in the shape `walk::Jsonl` writes it. Rows that do not
/// match are an error of their own — a trace of a different shape must be
/// recognised, never silently measured.
#[derive(Debug, Deserialize)]
struct Row {
    /// Monotonic milliseconds since the connection attempt.
    t_ms: f64,
    /// Protocol state of the row.
    state: String,
    /// `"serverbound"` / `"clientbound"`.
    dir: String,
    /// Packet name, absent when the trace has only the raw id.
    name: Option<String>,
    /// Standard base64 payload (padding included); the size is measured
    /// from it, the bytes themselves are not needed.
    b64: Option<String>,
}

/// Every entry-speed fact of one preload run. Times are absolute monotonic
/// milliseconds since the connect; subtract the `intention` of
/// [`Self::intention_ms`] to get the spans the owner asked for (they are
/// also available as methods).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PreloadMetrics {
    /// Serverbound `intention`: where every measurement starts.
    pub intention_ms: Option<f64>,
    /// Clientbound login (play).
    pub login_ms: Option<f64>,
    /// First `level_chunk_with_light`.
    pub first_chunk_ms: Option<f64>,
    /// The N-th `level_chunk_with_light` (the wait's target), when the
    /// server sent `N` chunks at all.
    pub target_chunk_ms: Option<f64>,
    /// Last `chunk_batch_finished` recorded in the trace.
    pub last_batch_ms: Option<f64>,
    /// Last `level_chunk_with_light` recorded in the trace.
    pub last_chunk_ms: Option<f64>,
    /// `level_chunk_with_light` packets seen.
    pub chunks: u64,
    /// `chunk_batch_start` packets seen.
    pub batch_starts: u64,
    /// `chunk_batch_finished` packets seen (one `chunk_batch_received` is
    /// sent back for each of them by the bot).
    pub batch_finishes: u64,
    /// Payload bytes of every `level_chunk_with_light`.
    pub chunk_bytes: u64,
    /// Payload bytes of every serverbound packet the bot sent (the acks).
    pub serverbound_bytes: u64,
    /// Chunks in each finished batch, in order (`size[i]` belongs to
    /// `batch_finishes[i]`).
    pub batch_sizes: Vec<u64>,
    /// Milliseconds between consecutive `chunk_batch_finished` packets.
    pub batch_gaps_ms: Vec<f64>,
    /// Lines that were blank (the writer ends a row with an empty line). A line
    /// that is neither blank nor the expected JSONL shape stops the reading
    /// with [`PreloadError::Json`] instead of being counted here.
    pub blank_lines: u64,
}

impl PreloadMetrics {
    /// Reads one trace written by the walk bot.
    ///
    /// `target_chunks` is the N of "the N-th chunk" metric; it never stops
    /// the reading — a trace simply has no such metric when the server
    /// stopped short of `N`.
    pub fn from_trace(path: &Path, target_chunks: u64) -> Result<Self, PreloadError> {
        let display = path.display().to_string();
        let file = std::fs::File::open(path).map_err(|source| PreloadError::Read {
            path: display.clone(),
            source,
        })?;
        let reader = std::io::BufReader::new(file);
        use std::io::BufRead as _;

        let mut metrics = Self::default();
        let mut since_finish = 0u64;
        let mut last_finish: Option<f64> = None;
        for (index, line) in reader.lines().enumerate() {
            let line_no = index + 1;
            let line = line.map_err(|source| PreloadError::Read {
                path: display.clone(),
                source,
            })?;
            if line.trim().is_empty() {
                metrics.blank_lines += 1;
                continue;
            }
            let row = serde_json::from_str::<Row>(&line).map_err(|source| PreloadError::Json {
                path: display.clone(),
                line: line_no,
                source,
            })?;
            let name = row.name.as_deref().unwrap_or_default();
            let bytes = row.b64.as_deref().map_or(0, b64_bytes);
            if row.dir == "serverbound" {
                metrics.serverbound_bytes += bytes;
                if name == "minecraft:intention" {
                    metrics.intention_ms = Some(row.t_ms);
                }
                continue;
            }
            match name {
                "minecraft:login" if row.state == "play" => metrics.login_ms = Some(row.t_ms),
                "minecraft:level_chunk_with_light" => {
                    metrics.chunks += 1;
                    metrics.chunk_bytes += bytes;
                    since_finish += 1;
                    metrics.first_chunk_ms = Some(metrics.first_chunk_ms.unwrap_or(row.t_ms));
                    metrics.last_chunk_ms = Some(row.t_ms);
                    if metrics.chunks == target_chunks {
                        metrics.target_chunk_ms = Some(row.t_ms);
                    }
                }
                "minecraft:chunk_batch_start" => {
                    metrics.batch_starts += 1;
                    since_finish = 0;
                }
                "minecraft:chunk_batch_finished" => {
                    metrics.batch_finishes += 1;
                    metrics.batch_sizes.push(since_finish);
                    if let Some(previous) = last_finish {
                        metrics.batch_gaps_ms.push(row.t_ms - previous);
                    }
                    last_finish = Some(row.t_ms);
                    metrics.last_batch_ms = Some(row.t_ms);
                }
                _ => {}
            }
        }
        Ok(metrics)
    }

    /// Span `intention → login` in milliseconds, or `None` when one of the
    /// two packets never arrived.
    pub fn intention_to_login_ms(&self) -> Option<f64> {
        span(self.intention_ms, self.login_ms)
    }

    /// Span `intention → first level_chunk_with_light`.
    pub fn intention_to_first_chunk_ms(&self) -> Option<f64> {
        span(self.intention_ms, self.first_chunk_ms)
    }

    /// Span `intention → last chunk_batch_finished`.
    pub fn intention_to_last_batch_ms(&self) -> Option<f64> {
        span(self.intention_ms, self.last_batch_ms)
    }

    /// Span `intention → the N-th chunk`: the target of the wait.
    pub fn intention_to_target_ms(&self) -> Option<f64> {
        span(self.intention_ms, self.target_chunk_ms)
    }

    /// Milliseconds the server needed from login to the target chunk.
    pub fn target_ms(&self) -> Option<f64> {
        span(self.login_ms, self.target_chunk_ms)
    }
}

/// Difference of two monotonic stamps, `None` unless both are known.
fn span(from: Option<f64>, to: Option<f64>) -> Option<f64> {
    Some(to? - from?)
}

/// Payload size a standard base64 row stands for (padding subtracted).
fn b64_bytes(text: &str) -> u64 {
    let chars = text.len() as u64;
    let padding = text.bytes().filter(|byte| *byte == b'=').count() as u64;
    chars / 4 * 3 - padding
}

/// `(min, median, max)` of every **present** measurement of one metric —
/// the three repetitions the owner asked for, or fewer if a run failed.
///
/// An odd count picks the middle value; an even count averages the two
/// middle ones. Empty input is `None`: no run is not a value.
pub fn min_median_max(values: &[Option<f64>]) -> Option<(f64, f64, f64)> {
    let mut sorted: Vec<f64> = values.iter().copied().flatten().collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("measured times are finite"));
    let max = *sorted.last().expect("not empty");
    let min = sorted[0];
    let median = if sorted.len() % 2 == 1 {
        sorted[sorted.len() / 2]
    } else {
        let upper = sorted[sorted.len() / 2];
        let lower = sorted[sorted.len() / 2 - 1];
        (upper + lower) / 2.0
    };
    Some((min, median, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a trace in exactly the shape `walk::Jsonl` writes it —
    /// including the blank line every row is followed by.
    fn write_trace<S: AsRef<str>>(rows: &[S]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "minerust-preload-{}.jsonl",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let mut text = String::new();
        for row in rows {
            text.push_str(row.as_ref());
            text.push('\n');
            text.push('\n');
        }
        std::fs::write(&path, &text).expect("write fixture");
        path
    }

    /// One row of a trace line, with the fields the reader looks at.
    fn row(t_ms: f64, state: &str, dir: &str, name: &str, b64: &str) -> String {
        let b64 = if b64.is_empty() {
            "null".to_owned()
        } else {
            format!("\"{b64}\"")
        };
        format!(
            "{{\"seq\":1,\"t_ms\":{t_ms},\"dir\":\"{dir}\",\"state\":\"{state}\",\
             \"id\":0,\"name\":\"{name}\",\"b64\":{b64}}}"
        )
    }

    #[test]
    fn an_intention_to_login_span_is_taken_from_the_trace() {
        let rows = [
            row(
                3.5,
                "handshake",
                "serverbound",
                "minecraft:intention",
                "AAAA",
            ),
            row(1425.6, "play", "clientbound", "minecraft:login", "AAAA"),
            row(
                1549.4,
                "play",
                "clientbound",
                "minecraft:level_chunk_with_light",
                "AQID",
            ),
            row(
                1800.0,
                "play",
                "clientbound",
                "minecraft:chunk_batch_start",
                "",
            ),
            row(
                1802.0,
                "play",
                "clientbound",
                "minecraft:chunk_batch_finished",
                "",
            ),
        ];
        let path = write_trace(&rows);

        let metrics = PreloadMetrics::from_trace(&path, 441).expect("parsed");
        assert_eq!(metrics.intention_ms, Some(3.5));
        assert_eq!(metrics.login_ms, Some(1425.6));
        assert_eq!(metrics.first_chunk_ms, Some(1549.4));
        assert_eq!(metrics.last_batch_ms, Some(1802.0));
        let login_span = metrics.intention_to_login_ms().expect("both packets seen");
        assert!((login_span - 1422.1).abs() < 1e-9, "{login_span}");
        assert_eq!(
            metrics.intention_to_first_chunk_ms(),
            Some(1549.4 - 3.5),
            "the span starts at `intention`, not at `login`"
        );
        assert_eq!(metrics.target_chunk_ms, None, "441 chunks were not here");
        assert_eq!(metrics.chunks, 1);
        assert_eq!(metrics.chunk_bytes, 3, "base64 `AQID` is three bytes");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn the_target_chunk_is_the_nth_and_the_blanks_are_not_rows() {
        let mut rows = vec![row(
            1.0,
            "handshake",
            "serverbound",
            "minecraft:intention",
            "",
        )];
        for index in 1..=3_u8 {
            rows.push(row(
                100.0 + f64::from(index),
                "play",
                "clientbound",
                "minecraft:level_chunk_with_light",
                "",
            ));
        }
        // `login` here is decked *after* the chunks, on purpose: the target
        // metric must not care about the surrounding order.
        rows.push(row(500.0, "play", "clientbound", "minecraft:login", ""));
        let path = write_trace(&rows);

        let metrics = PreloadMetrics::from_trace(&path, 3).expect("parsed");
        assert_eq!(metrics.target_chunk_ms, Some(103.0), "the 3rd chunk");
        assert_eq!(metrics.chunks, 3);
        // The writer ends every row with an extra blank line: five rows of
        // this fixture, five blank lines of it.
        assert_eq!(metrics.blank_lines, 5, "one blank per row");

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_row_that_is_not_the_walk_jsonl_is_an_error_not_a_zero() {
        let path = write_trace(&["this is not json"]);
        let error = PreloadMetrics::from_trace(&path, 441).expect_err("must not parse");
        assert!(
            matches!(error, PreloadError::Json { line: 1, .. }),
            "{error}"
        );

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_missing_trace_is_reported_by_name() {
        let error = PreloadMetrics::from_trace(Path::new("definitely-not-here.jsonl"), 441)
            .expect_err("no file");
        assert!(error.to_string().contains("definitely-not-here"), "{error}");
    }

    #[test]
    fn min_median_max_needs_at_least_one_value() {
        assert_eq!(min_median_max(&[]), None);
        assert_eq!(min_median_max(&[None, None]), None);
        assert_eq!(min_median_max(&[Some(3.0)]), Some((3.0, 3.0, 3.0)));
        assert_eq!(
            min_median_max(&[Some(9.0), None, Some(1.0), Some(5.0)]),
            Some((1.0, 5.0, 9.0)),
            "an absent run is skipped, not counted as zero"
        );
        assert_eq!(
            min_median_max(&[Some(1.0), Some(9.0)]),
            Some((1.0, 5.0, 9.0)),
            "an even count averages the two middle values"
        );
    }
}
