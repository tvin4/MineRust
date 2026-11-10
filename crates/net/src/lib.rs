//! Tokio networking layer: accepts TCP connections and drives the
//! handshake → status, login, configuration and play state machines used
//! by the server list ping and by a player joining the world.
//!
//! The crate is transport-only: the contents of the status response are
//! decided by the caller (see the `server` crate).

pub mod chunks;
mod connection;
mod error;
mod frame;
pub mod game;
pub mod loader;
pub mod trace;

pub use chunks::{window, window_difference, ReadyChunks};
pub use connection::{serve, serve_with, serve_with_metrics, WorldInfo};
pub use error::ConnectionError;
pub use frame::{read_frame, write_frame};
pub use loader::{ChunkSource, Loader};
