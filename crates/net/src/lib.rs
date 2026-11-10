//! Tokio networking layer: accepts TCP connections and drives the
//! handshake → status, login, configuration and play state machines used
//! by the server list ping and by a player joining the world.
//!
//! The crate is transport-only: the contents of the status response are
//! decided by the caller (see the `server` crate).

mod connection;
mod error;
mod frame;
pub mod game;
pub mod loader;
pub mod trace;

pub use connection::{serve, serve_with, WorldInfo};
pub use error::ConnectionError;
pub use frame::{read_frame, write_frame};
pub use loader::{ChunkSource, Loader};
