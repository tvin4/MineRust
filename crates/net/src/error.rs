//! Errors produced while serving a single connection.

use protocol::CodecError;

/// Errors while serving a single connection.
#[derive(Debug, thiserror::Error)]
pub enum ConnectionError {
    /// Transport-level I/O failure; may simply be the client going away.
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    /// The peer sent malformed protocol data.
    #[error(transparent)]
    Codec(#[from] CodecError),
    /// A packet id was not valid in the current connection state.
    #[error("unexpected packet id {id} in state {state}")]
    UnexpectedPacket {
        /// Packet id that was received.
        id: u32,
        /// Name of the connection state where the packet arrived.
        state: &'static str,
    },
    /// The connection did not reach the play state within the handshake
    /// timeout (the play state itself is not subject to a global timeout).
    #[error("timed out before reaching play state")]
    TimedOut,
    /// The operating system could not provide random bytes for the
    /// per-session UUID of Login Success (`getrandom::Error` has a `Display`
    /// but intentionally does not implement `std::error::Error`).
    #[error("no system entropy for the session uuid: {0}")]
    Entropy(String),
}

impl ConnectionError {
    /// Returns `true` when the error is just the client disconnecting.
    pub fn is_client_disconnect(&self) -> bool {
        matches!(
            self,
            Self::Io(io) if matches!(
                io.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::BrokenPipe
            )
        )
    }
}
