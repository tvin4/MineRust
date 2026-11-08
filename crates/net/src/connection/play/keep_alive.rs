//! The keep-alive of one play connection: interval, timeout watchdog and
//! the id sent with every tick.

use std::time::Duration;

use tokio::time::Instant;

/// How often the server asks the client for a keep-alive response in play.
///
/// SOURCE: parity recording of vanilla 26.3 — the two clientbound
/// keep-alives of the recorded join are 15,004 ms apart (ids
/// `0x02206633` and `0x0220A0CF`, milliseconds of a monotonic clock — their
/// magnitude matches the machine's uptime at the time of the run).
pub(super) const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// A connection is dropped when this much time passes after the sent
/// keep-alive without an answer from the client.
// SOURCE: wiki, unverified — a vanilla timeout is not observable in a
// recording where the scripted client always answers; only the interval
// above is measured.
const KEEP_ALIVE_TIMEOUT: Duration = Duration::from_secs(30);

/// Tracks the keep-alive outstanding on one connection: the timestamp it
/// was sent at, cleared when the client echoes the id back. When
/// [`KEEP_ALIVE_TIMEOUT`] passes without an answer the connection ends.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct KeepAliveWatch {
    /// When the unanswered keep-alive was sent.
    sent: Option<Instant>,
}

impl KeepAliveWatch {
    /// Records a freshly sent keep-alive.
    pub(super) fn on_sent(&mut self, now: Instant) {
        self.sent = Some(now);
    }

    /// The client answered; nothing is outstanding.
    pub(super) fn on_acked(&mut self) {
        self.sent = None;
    }

    /// Deadline of the outstanding keep-alive, if there is one.
    pub(super) fn deadline(&self) -> Option<Instant> {
        self.sent.map(|sent| sent + KEEP_ALIVE_TIMEOUT)
    }
}

/// Waits until `deadline`; `None` never resolves (the select arm is
/// guarded by the same option, this only keeps the future non-empty).
pub(super) async fn wait_for_deadline(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// Milliseconds of a monotonic clock, the keep-alive id (vanilla's id is
/// milliseconds of its own monotonic clock too — observed as the machine's
/// uptime at recording time; ours is anchored at server start, which is the
/// same class of value and never byte-comparable anyway).
// SOURCE: parity recording of vanilla 26.3 (origin of our anchor differs).
pub(super) fn keep_alive_millis() -> i64 {
    static ANCHOR: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let anchor = ANCHOR.get_or_init(Instant::now);
    i64::try_from(
        Instant::now()
            .saturating_duration_since(*anchor)
            .as_millis(),
    )
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sent keep-alive arms a deadline exactly `KEEP_ALIVE_TIMEOUT` later,
    /// an answer disarms it, and no keep-alive means no deadline at all.
    #[test]
    fn keep_alive_watch_tracks_the_outstanding_keep_alive() {
        let mut watch = KeepAliveWatch::default();
        assert_eq!(watch.deadline(), None, "nothing outstanding at first");

        let sent = Instant::now();
        watch.on_sent(sent);
        assert_eq!(watch.deadline(), Some(sent + KEEP_ALIVE_TIMEOUT));

        watch.on_acked();
        assert_eq!(watch.deadline(), None, "the answer clears the deadline");

        // A second round starts from a fresh timestamp.
        let sent = Instant::now() + Duration::from_secs(5);
        watch.on_sent(sent);
        assert_eq!(watch.deadline(), Some(sent + KEEP_ALIVE_TIMEOUT));
    }

    /// `wait_for_deadline` waits for a real deadline and stays pending
    /// without one (the guarded select arm must never fire).
    #[tokio::test]
    async fn wait_for_deadline_sleeps_until_the_deadline() {
        let past = Instant::now() - Duration::from_secs(1);
        tokio::time::timeout(Duration::from_secs(1), wait_for_deadline(Some(past)))
            .await
            .expect("a passed deadline resolves immediately");
        tokio::time::timeout(Duration::from_millis(50), wait_for_deadline(None))
            .await
            .expect_err("no deadline must never resolve");
    }

    /// The keep-alive id is milliseconds of a monotonic clock and never
    /// goes backwards between two calls.
    #[test]
    fn keep_alive_id_is_monotonic_milliseconds() {
        let first = keep_alive_millis();
        let second = keep_alive_millis();
        assert!(first >= 0, "milliseconds since server start");
        assert!(second >= first, "a monotonic clock never goes back");
    }
}
