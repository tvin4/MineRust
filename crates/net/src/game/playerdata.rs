//! The player save runtime the game thread and the connections share (Шаг 18a).
//!
//! One [`Store`] lives for the whole run: the connection reads a player file
//! at join (through `spawn_blocking`, never on the tick) and the game thread
//! queues snapshots for the writer thread of the `playerdata` crate. The tick
//! never touches the disk: it builds a [`Snapshot`] and hands it to the
//! runtime's [`Sink`].

use std::sync::Arc;

use playerdata::{Pose, SavedPlayer, Sink, Snapshot, Store};
use protocol::Uuid;
use tracing::warn;

use super::messages::PlayerId;
use super::region::Player;
use super::Scheduler;

/// Everything one run needs to persist its players.
#[derive(Debug, Clone)]
pub struct PlayerdataRuntime {
    /// Open save store: reads at join, file naming, the data schema.
    pub store: Arc<Store>,
    /// Queue to the dedicated writer thread (never blocks the tick).
    pub sink: Sink,
    /// When true the configured game mode always wins on join; when false a
    /// saved `playerGameType` is used if the file carries one (Шаг 18a, the
    /// owner's decision).
    pub force_gamemode: bool,
    /// Autosave period in ticks; `0` disables the periodic save (the quit and
    /// shutdown saves still run).
    pub autosave_interval_ticks: u64,
}

impl PlayerdataRuntime {
    /// Reads one player's file; a broken file is logged and read as "no file"
    /// (the player joins as new).
    ///
    /// Disk work: the connection calls this from `spawn_blocking`.
    pub fn read(&self, uuid: Uuid) -> Option<SavedPlayer> {
        match self.store.read(uuid) {
            Ok(saved) => saved,
            Err(err) => {
                warn!(%err, "could not read the player file, joining as a new player");
                None
            }
        }
    }
}

/// Builds the snapshot of one player for a save.
pub fn snapshot_of(player: &Player) -> Snapshot {
    let position = player.position();
    Snapshot {
        uuid: player.uuid(),
        pose: Pose {
            x: position.x,
            y: position.y,
            z: position.z,
            yaw: position.yaw,
            pitch: position.pitch,
        },
        on_ground: position.on_ground(),
        health: player.health(),
        food: player.food(),
        saturation: player.saturation(),
        game_mode: player.game_mode(),
    }
}

/// Queues one player's snapshot (a quit, Шаг 18a).
pub fn save_player(scheduler: &Scheduler, runtime: &PlayerdataRuntime, id: PlayerId) {
    let Some(player) = scheduler.player(id) else {
        return;
    };
    queue(runtime, player.name(), snapshot_of(player));
}

/// Queues every online player's snapshot (autosave and shutdown).
pub fn save_all(scheduler: &Scheduler, runtime: &PlayerdataRuntime) {
    for player in scheduler.players() {
        queue(runtime, player.name(), snapshot_of(player));
    }
}

/// Hands one snapshot to the writer queue; a full queue is logged, never a
/// tick stall (the next autosave writes the newer state anyway).
fn queue(runtime: &PlayerdataRuntime, name: &str, snapshot: Snapshot) {
    if !runtime.sink.snapshot(snapshot) {
        warn!(
            player = name,
            "the player save queue is full, snapshot dropped"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::test_support::{join_at, join_at_mode};
    use crate::game::{MoveUpdate, Player, PlayerJoin, PlayerPos, Region, SINGLE_REGION};
    use playerdata::Store;
    use std::sync::Arc;
    use std::time::Duration;

    /// A unique directory under the system temp root.
    fn temp_dir(name: &str) -> std::path::PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "minerust-save-{name}-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// A runtime over a throwaway store plus a reader and the writer handle.
    fn runtime(name: &str) -> (PlayerdataRuntime, Store, playerdata::Writer) {
        let store = Store::open(temp_dir(name), 5023, &[]).expect("store");
        let reader = store.clone();
        let (sink, writer) = playerdata::spawn(store.clone(), 8).expect("writer");
        let runtime = PlayerdataRuntime {
            store: Arc::new(store),
            sink,
            force_gamemode: true,
            autosave_interval_ticks: 0,
        };
        (runtime, reader, writer)
    }

    /// A scheduler with the world in one region and no pool.
    fn scheduler() -> Scheduler {
        Scheduler::new(vec![Region::new(SINGLE_REGION)])
    }

    #[test]
    fn a_quit_saves_the_player_to_its_file() {
        let (runtime, store, writer) = runtime("quit");
        let mut scheduler = scheduler();
        let (join, _slot, _rx) = join_at(1, (10, 64, 20));
        let name = join.name.clone();
        let join = PlayerJoin {
            pose: Some(PlayerPos {
                x: 10.5,
                y: 64.0,
                z: 20.5,
                yaw: 45.0,
                pitch: 10.0,
                head_yaw: 45.0,
                flags: MoveUpdate::ON_GROUND,
            }),
            health: 15.5,
            food: 17,
            saturation: 2.5,
            ..join
        };
        scheduler.insert_player(join);

        save_player(&scheduler, &runtime, 1);
        writer
            .shutdown(Duration::from_secs(10))
            .expect("writer stops");

        let saved = store
            .read(protocol::offline_uuid(&name))
            .expect("read")
            .expect("the quit wrote the file");
        assert_eq!(
            (saved.pose.x, saved.pose.y, saved.pose.z),
            (10.5, 64.0, 20.5)
        );
        assert_eq!((saved.pose.yaw, saved.pose.pitch), (45.0, 10.0));
        assert!(saved.on_ground, "the movement flag reached the file");
        assert_eq!(saved.health, 15.5);
        assert_eq!(saved.food, 17);
        assert_eq!(saved.saturation, 2.5);
        assert_eq!(saved.game_mode, Some(0));
        assert_eq!(saved.dimension, playerdata::OVERWORLD);
    }

    #[test]
    fn an_autosave_saves_every_online_player() {
        let (runtime, store, writer) = runtime("autosave");
        let mut scheduler = scheduler();
        let (first, _slot, _rx) = join_at(1, (0, 64, 0));
        let (second, _slot, _rx) = join_at(2, (32, 64, 32));
        let names = [first.name.clone(), second.name.clone()];
        scheduler.insert_player(first);
        scheduler.insert_player(second);

        save_all(&scheduler, &runtime);
        writer
            .shutdown(Duration::from_secs(10))
            .expect("writer stops");

        for name in names {
            assert!(
                store
                    .read(protocol::offline_uuid(&name))
                    .expect("read")
                    .is_some(),
                "{name} was saved"
            );
        }
    }

    #[test]
    fn a_snapshot_carries_the_player_state() {
        let (join, _slot, _rx) = join_at_mode(9, (1, 2, 3), 1);
        let name = join.name.clone();
        let player = Player::new(join);
        let snapshot = snapshot_of(&player);

        assert_eq!(snapshot.uuid, protocol::offline_uuid(&name));
        assert_eq!(
            (snapshot.pose.x, snapshot.pose.y, snapshot.pose.z),
            (1.5, 2.0, 3.5),
            "no saved pose: the spawn block's centre"
        );
        assert_eq!((snapshot.pose.yaw, snapshot.pose.pitch), (0.0, 0.0));
        assert!(!snapshot.on_ground, "the entry carries no movement flags");
        assert_eq!(snapshot.health, 20.0);
        assert_eq!(snapshot.food, 20);
        assert_eq!(snapshot.saturation, 5.0);
        assert_eq!(snapshot.game_mode, 1);
    }
}
