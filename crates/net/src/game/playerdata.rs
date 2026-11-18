//! The player save runtime the game thread and the connections share (Шаг 18a).
//!
//! One [`Store`] lives for the whole run: the connection reads a player file
//! at join (through `spawn_blocking`, never on the tick) and the game thread
//! queues snapshots for the writer thread of the `playerdata` crate. The tick
//! never touches the disk: it builds a [`Snapshot`] and hands it to the
//! runtime's [`Sink`].

use std::sync::{Arc, Mutex};

use playerdata::{Pose, SavedPlayer, Sink, Snapshot, Store};
use protocol::Uuid;
use tracing::warn;

use super::messages::PlayerId;
use super::region::Player;
use super::Scheduler;

/// Everything one run needs to persist its players.
#[derive(Debug)]
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
    /// Final online-player snapshots staged after the tick loop. The server
    /// queues them only after the final world flush, so the newest player
    /// position cannot overtake the world it belongs to (Шаг 22b).
    ///
    /// This lock is used exactly twice during shutdown (replace, then take),
    /// never on a running tick or connection path.
    final_snapshots: Mutex<Vec<(String, Snapshot)>>,
}

impl PlayerdataRuntime {
    /// Builds one runtime over the already opened store and writer sink.
    pub fn new(
        store: Arc<Store>,
        sink: Sink,
        force_gamemode: bool,
        autosave_interval_ticks: u64,
    ) -> Self {
        Self {
            store,
            sink,
            force_gamemode,
            autosave_interval_ticks,
            final_snapshots: Mutex::new(Vec::new()),
        }
    }

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

    /// Queues the snapshots staged at game-thread shutdown, after the caller
    /// has made the final world durable. Returns `(queued, dropped)`.
    pub fn flush_final(&self) -> (usize, usize) {
        let snapshots = {
            let mut staged = self
                .final_snapshots
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *staged)
        };
        let total = snapshots.len();
        let mut queued = 0;
        for (name, snapshot) in snapshots {
            if queue(self, &name, snapshot) {
                queued += 1;
            }
        }
        // `queue` does not put a rejected snapshot back: report it so the
        // server never calls the final player flush successful by mistake.
        (queued, total - queued)
    }
}

/// Builds the snapshot of one player for a save.
///
/// `items` names the item registry so the hotbar's protocol ids can be written
/// back as identifiers (Шаг 24d); without it the saved inventory is empty.
pub fn snapshot_of(player: &Player, items: Option<&data::RegistryIds>) -> Snapshot {
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
        inventory: player.inventory().saved_hotbar(items),
    }
}

/// Queues one player's snapshot (a quit, Шаг 18a).
pub fn save_player(
    scheduler: &Scheduler,
    runtime: &PlayerdataRuntime,
    id: PlayerId,
    items: Option<&data::RegistryIds>,
) {
    let Some(player) = scheduler.player(id) else {
        return;
    };
    queue(runtime, player.name(), snapshot_of(player, items));
}

/// Queues every online player's snapshot (autosave and shutdown).
pub fn save_all(
    scheduler: &Scheduler,
    runtime: &PlayerdataRuntime,
    items: Option<&data::RegistryIds>,
) {
    for player in scheduler.players() {
        queue(runtime, player.name(), snapshot_of(player, items));
    }
}

/// Captures every online player after the final tick, but does not hand the
/// snapshots to the disk writer yet. The server flushes them after the world.
pub fn stage_all(
    scheduler: &Scheduler,
    runtime: &PlayerdataRuntime,
    items: Option<&data::RegistryIds>,
) {
    let snapshots = scheduler
        .players()
        .map(|player| (player.name().to_owned(), snapshot_of(player, items)))
        .collect();
    let mut staged = runtime
        .final_snapshots
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    *staged = snapshots;
}

/// Hands one snapshot to the writer queue; a full queue is logged, never a
/// tick stall (the next autosave writes the newer state anyway).
fn queue(runtime: &PlayerdataRuntime, name: &str, snapshot: Snapshot) -> bool {
    let queued = runtime.sink.snapshot(snapshot);
    if !queued {
        warn!(
            player = name,
            "the player save queue is full, snapshot dropped"
        );
    }
    queued
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
        let runtime = PlayerdataRuntime::new(Arc::new(store), sink, true, 0);
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
            saved_inventory: Vec::new(),
            ..join
        };
        scheduler.insert_player(join);

        save_player(&scheduler, &runtime, 1, None);
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

        save_all(&scheduler, &runtime, None);
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

    /// Shutdown snapshots stay in memory until the server confirms the world
    /// flush, then enter the player writer together.
    #[test]
    fn final_snapshots_are_staged_before_their_writer_queue() {
        let (runtime, store, writer) = runtime("staged-final");
        let mut scheduler = scheduler();
        let (first, _slot, _rx) = join_at(1, (0, 64, 0));
        let name = first.name.clone();
        let uuid = protocol::offline_uuid(&name);
        scheduler.insert_player(first);

        stage_all(&scheduler, &runtime, None);
        assert!(
            store.read(uuid).expect("read before flush").is_none(),
            "staging alone does not let playerdata overtake the world"
        );
        assert_eq!(runtime.flush_final(), (1, 0));
        writer
            .shutdown(Duration::from_secs(10))
            .expect("writer stops");
        assert!(
            store.read(uuid).expect("read after flush").is_some(),
            "the server-side flush queues the staged snapshot"
        );
    }

    #[test]
    fn a_snapshot_carries_the_player_state() {
        let (join, _slot, _rx) = join_at_mode(9, (1, 2, 3), 1);
        let name = join.name.clone();
        let player = Player::new(join);
        let snapshot = snapshot_of(&player, None);

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

    #[test]
    fn a_gamemode_command_change_reaches_the_saved_snapshot() {
        // Шаг 25c: `/gamemode` mutates the tick-side player, and the save
        // snapshot reads that field, so the new mode is what the next join
        // sees (subject to `force_gamemode`, covered in the play tests).
        let (join, _slot, _rx) = join_at_mode(9, (0, 64, 0), 0);
        let mut player = Player::new(join);
        assert!(player.set_game_mode(3), "survival to spectator changes it");
        assert!(!player.set_game_mode(3), "setting the same mode is a no-op");
        let snapshot = snapshot_of(&player, None);
        assert_eq!(snapshot.game_mode, 3);
    }
}
