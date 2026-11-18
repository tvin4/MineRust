//! Vanilla-compatible player save files (Шаг 18a).
//!
//! One world keeps one [`Store`] rooted at the configured `save_dir`
//! (`<save_dir>/players/data/<uuid>.dat`, the vanilla layout). The format is
//! the vanilla one: a gzip container around the disk-NBT form of a named
//! compound. The store reads the fields the server models (position,
//! rotation, health, food, saturation, game mode) and writes only those back,
//! keeping every unknown tag of an existing file.
//!
//! Writing is disk work and never runs on the tick thread: the game thread
//! builds a [`Snapshot`] and hands it to a [`Sink`], a dedicated writer
//! thread ([`spawn`]) consumes the queue in order and [`Writer::shutdown`]
//! drains it at server stop.
//!
//! SOURCE: the owner's own world saves and the S7 join trace
//! (`docs/PLAYERDATA.md` §1); field names and shapes were read out of real
//! `.dat` files, never copied from the game's code.

mod error;
mod state;
mod store;
mod writer;

pub use error::PlayerdataError;
pub use state::{
    Pose, SavedItem, SavedPlayer, Snapshot, NEW_PLAYER_FOOD, NEW_PLAYER_HEALTH,
    NEW_PLAYER_SATURATION, OVERWORLD,
};
pub use store::{file_name, Store};
pub use writer::{spawn, Sink, Writer, DEFAULT_CAPACITY};
