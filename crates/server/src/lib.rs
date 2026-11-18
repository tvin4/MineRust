//! Server library: configuration loading and the startup sequence.
//!
//! The binary in `main.rs` is a thin wrapper around [`run`]. Keeping the
//! logic in a library allows the integration tests to start a real server on
//! an ephemeral port without spawning a subprocess.

mod config;
mod geometry;
mod template;

pub use config::{load_config, ConfigError, ServerConfig};
/// The saved-world reader and its id tables, exported for the parity crate:
/// the chunk slice (step 3.3f) builds the very payloads this server would
/// send and diffs them against the recorded vanilla ones.
pub use template::{TemplateWorld, WireTables};

use std::path::Path;
use std::sync::Arc;

use data::{BlockHardness, BlockStates, CollisionShapes, GameData};
use net::game::WorldSave as _;
use protocol::StatusInfo;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

/// Builds the status document advertised to the multiplayer server list.
///
/// The player count is hard-coded to 0 for now: tracking online players
/// belongs to milestone M2 (login).
pub fn status_info(config: &ServerConfig) -> StatusInfo {
    StatusInfo {
        version_name: config.version_name.clone(),
        protocol_version: config.protocol_version,
        max_players: config.max_players,
        online_players: 0,
        motd: config.motd.clone(),
    }
}

/// Errors of the startup sequence: config-time I/O and game data problems.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The listener could not be bound or a connection layer I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Game data (jar, reports) could not be read or did not verify.
    #[error(transparent)]
    Data(#[from] data::DataError),
    /// The world template (`level.dat`, region layout) could not be read.
    #[error(transparent)]
    World(#[from] world::WorldError),
    /// The configured world does not have the shape a save has.
    #[error("invalid world_dir: {0}")]
    InvalidWorld(String),
    /// A chunk-serving path is missing from `server.toml`. The startup
    /// refuses to run without it — a server that starts silently and then
    /// sends no chunks leaves vanilla clients waiting for terrain until
    /// they time out and fall into the void (the live check of step 3.3).
    #[error("`{setting}` is missing from server.toml: {hint}")]
    MissingSetting {
        /// Configuration field that is not set.
        setting: &'static str,
        /// What the field must point at.
        hint: &'static str,
    },
    /// The development geometry file could not be read, parsed or applied
    /// (Шаг 15 п.2).
    #[error(transparent)]
    Geometry(#[from] geometry::GeometryError),
    /// The player save store could not be opened (Шаг 18a): a bad path (the
    /// world template or the vanilla stand), an unwritable directory or a
    /// writer thread that could not start.
    #[error(transparent)]
    Playerdata(#[from] playerdata::PlayerdataError),
    /// The world save could not be opened (Шаг 22b): the save belongs to
    /// another template, a directory or file could not be read or written,
    /// or the writer thread could not start.
    #[error(transparent)]
    Worldsave(#[from] worldsave::StoreError),
    /// The template or its saved diffs could not be built before bind.
    #[error("world warm-up failed: {0}")]
    Warmup(String),
    /// The game thread could still mutate the world after its join deadline,
    /// so no racy final snapshot was attempted.
    #[error("the game thread did not stop; final world/player saves were not attempted")]
    GameShutdown,
    /// The final player snapshots were staged in the right order but the
    /// bounded writer queue could not accept all of them.
    #[error("the final player-save queue dropped {0} snapshot(s)")]
    FinalPlayerQueue(usize),
}

/// What each required chunk-serving path must point at (see
/// [`require_chunk_paths`]).
const WORLD_HINT: &str = "point it at a saved world: the directory with \
    `level.dat` and `dimensions/minecraft/overworld/region/`; without a world \
    the server has no chunks to send (see server.example.toml)";
const REPORT_HINT: &str = "point it at the report of your game copy, generated \
    by tools\\generate-reports.ps1 (see server.example.toml)";

/// Fails the startup when `server.toml` does not name every path the chunk
/// sending of the live check needs: the world itself and the three reports
/// its wire payloads are encoded from.
///
/// Checked before anything is loaded so a wrong configuration is one clear
/// message on stderr, never a server that runs without terrain. The check
/// only looks at presence — unreadable files are reported by their own
/// loader with the failing path.
pub fn require_chunk_paths(config: &ServerConfig) -> Result<(), RunError> {
    let required = [
        ("world_dir", WORLD_HINT, &config.world_dir),
        ("blocks_report", REPORT_HINT, &config.blocks_report),
        ("registries_report", REPORT_HINT, &config.registries_report),
        ("packets_report", REPORT_HINT, &config.packets_report),
    ];
    for (setting, hint, value) in required {
        let present = value
            .as_deref()
            .map(str::trim)
            .is_some_and(|path| !path.is_empty());
        if !present {
            return Err(RunError::MissingSetting { setting, hint });
        }
    }
    Ok(())
}

/// Loads the packet ids the server will speak.
///
/// With `config.packets_report` set the ids come from the data generator's
/// report of the user's own game copy and every packet key must be covered;
/// without it the compiled defaults are used (SOURCE: wiki, unverified) and
/// the caller is told so by a warning.
fn load_packet_ids(config: &ServerConfig) -> Result<Arc<data::PacketIds>, RunError> {
    let Some(path) = config.packets_report.as_deref() else {
        tracing::warn!(
            "no packets_report configured, using packet ids from the public \
             protocol documentation (unverified); run tools/generate-reports.ps1"
        );
        return Ok(Arc::new(data::PacketIds::compiled()));
    };
    let ids = data::load_packets_report(path)?;
    ids.verify(protocol::used_packet_keys())?;
    tracing::info!(
        path,
        ids = ids.len(),
        "packet ids loaded from the packets report"
    );
    Ok(Arc::new(ids))
}

/// Loads the collision-shape report that drives the free-headroom query of the
/// forced crouch/crawl rule (Шаг 15 п.5).
///
/// The setting is **optional**: without `collision_report` the rule is
/// disabled — the store answers `None` for every headroom ([`data::CollisionShapes`]
/// stays empty and `net`'s `ChunkStore::free_headroom` returns `None`), so the
/// player's pose follows its own input exactly as it did before Шаг 15. The
/// warning is the only sign, so it names the setting and the rule it switches
/// off; the server itself runs unchanged.
///
/// SOURCE: `reports/collision.json`, generated by the test client's `shapes`
/// command from the user's own game copy (see `tools/testclient/README.md`).
fn load_collisions(config: &ServerConfig) -> Result<CollisionShapes, RunError> {
    let Some(path) = config.collision_report.as_deref() else {
        tracing::warn!(
            "no collision_report: the forced crouch/crawl pose (Шаг 15 п.5) is off; \
             set collision_report=<path>, generate the file with the test client's \
             `shapes` command"
        );
        return Ok(CollisionShapes::default());
    };
    let collisions = CollisionShapes::load(path)?;
    tracing::info!(path, states = collisions.len(), "collision report loaded");
    Ok(collisions)
}

/// Loads the registries report a second time for the game thread (Шаг 24a/24d).
///
/// The world template already reads it for its own id lookups (`WireTables`),
/// but the game thread needs the item ids for the creative inventory and the
/// item names for the player file. The file is small and read once at
/// start-up, so a separate copy is cheaper than threading the template's into
/// the game thread. A configured but broken report fails the start, like the
/// template's own read.
fn load_registries(config: &ServerConfig) -> Result<Option<Arc<data::RegistryIds>>, RunError> {
    let Some(path) = config.registries_report.as_deref() else {
        return Ok(None);
    };
    let ids = data::load_registries_report(path)?;
    tracing::info!(
        path,
        registries = ids.len(),
        "registries report loaded for the inventory"
    );
    Ok(Some(Arc::new(ids)))
}

/// Loads the hardness report the server-side mining timer reads (Шаг 16a).
///
/// The setting is **optional**: without `hardness_report` the timer has no
/// hardness to compare a dig against, so mining keeps working exactly as it
/// did before Шаг 16 — the server trusts the client's `player_action` finish.
/// The warning is the only sign, so it names the setting and what stays off;
/// the server itself runs unchanged. A report that *is* configured is loaded
/// and validated here, before the port is bound, so a broken file fails the
/// start instead of surfacing on the first dig.
///
/// SOURCE: `reports/hardness.json`, generated by the test client's `hardness`
/// command from the user's own game copy (see `tools/testclient/README.md`).
fn load_hardness(config: &ServerConfig) -> Result<Option<Arc<BlockHardness>>, RunError> {
    let Some(path) = config.hardness_report.as_deref() else {
        tracing::warn!(
            "no hardness_report: observer crack stages (Шаг 16a) are off; \
             set hardness_report=<path>, generate the file with the test client's \
             `hardness` command"
        );
        return Ok(None);
    };
    let hardness = BlockHardness::load(path)?;
    tracing::info!(path, states = hardness.len(), "hardness report loaded");
    Ok(Some(Arc::new(hardness)))
}

/// Builds the chunk loader pool and the join values (`net::WorldInfo`) of
/// the configured world: the save supplies spawn and hashed seed, the
/// configuration supplies `view_distance` and `simulation_distance`.
///
/// Every path is demanded by [`require_chunk_paths`] at the top of `run`
/// first, so `world_dir` is always set here — the `None` branch only stays
/// as the direct, library-level answer for a caller that skipped that
/// check (the unit tests and the parity self-session build the world above
/// the check). Everything happens before the port is bound: a world whose
/// `level.dat` or `world_gen_settings.dat` cannot be read or whose region
/// directory is missing fails at startup, never as a failed read on
/// someone's join.
pub fn build_world(
    config: &ServerConfig,
    game_data: Arc<GameData>,
) -> Result<(Option<net::Loader>, net::WorldInfo), RunError> {
    build_world_with_save(config, game_data, None)
}

/// [`build_world`] with the saved chunk diffs to lay over the template
/// (Шаг 22b).
///
/// The save is optional and explicit: the parity stands keep reading exactly
/// the template, while `run` passes the store opened from `save_dir`. The
/// diffs are applied when the batch is built (the warm-up), before any
/// connection can join.
pub fn build_world_with_save(
    config: &ServerConfig,
    game_data: Arc<GameData>,
    saved: Option<Arc<worldsave::Store>>,
) -> Result<(Option<net::Loader>, net::WorldInfo), RunError> {
    let Some(dir) = config.world_dir.as_deref().map(str::trim) else {
        // Only reachable when `run` was bypassed; `run` refuses to start
        // without a world (require_chunk_paths).
        tracing::warn!("no world_dir configured: the server sends no chunks");
        // The configured distances still reach Login (play): they belong
        // to the configuration, not to the save.
        return Ok((
            None,
            net::WorldInfo {
                view_distance: config.view_distance,
                simulation_distance: config.simulation_distance,
                ..net::WorldInfo::default()
            },
        ));
    };
    let root = Path::new(dir);
    let level = world::read_level_dat(root.join("level.dat"))?;
    let settings =
        world::read_world_gen_settings(root.join("data/minecraft/world_gen_settings.dat"))?;
    let region_dir = root.join("dimensions/minecraft/overworld/region");
    if !region_dir.is_dir() {
        return Err(RunError::InvalidWorld(format!(
            "{} is missing the dimension directory",
            root.display()
        )));
    }
    // Step 3.3e: the seed of the save travels into Login (play) hashed by
    // the formula the vanilla recording verified (parity test `login_fields`).
    // The two chunk distances come from `server.toml` (both default to the
    // vanilla 10 of the `login_fields` recording).
    let info = net::WorldInfo {
        spawn: level.spawn,
        player_spawn_override: None,
        hashed_seed: protocol::hashed_seed(settings.seed),
        view_distance: config.view_distance,
        simulation_distance: config.simulation_distance,
    };
    let blocks = match config.blocks_report.as_deref() {
        Some(path) => {
            let blocks = BlockStates::load(path)?;
            tracing::info!(path, states = blocks.len(), "blocks report loaded");
            blocks
        }
        None => {
            tracing::warn!("no blocks_report: chunk palettes cannot be resolved to state ids");
            BlockStates::default()
        }
    };
    let registries = match config.registries_report.as_deref() {
        Some(path) => data::load_registries_report(path)?,
        None => {
            tracing::warn!("no registries_report: block entity type ids cannot be resolved");
            data::RegistryIds::default()
        }
    };
    let collisions = load_collisions(config)?;
    let tables = Arc::new(WireTables::new(blocks, registries, game_data, collisions));
    let mut world = TemplateWorld::new(region_dir, level.data_version, tables);
    if let Some(store) = saved {
        tracing::info!(
            save_dir = %store.root().display(),
            mismatch = store.mismatch(),
            "saved chunk diffs will be applied over the template"
        );
        world = world.with_saved(store);
    }
    tracing::info!(
        data_version = level.data_version,
        seed = settings.seed,
        hashed_seed = info.hashed_seed,
        spawn_x = info.spawn.x,
        spawn_y = info.spawn.y,
        spawn_z = info.spawn.z,
        view_distance = info.view_distance,
        simulation_distance = info.simulation_distance,
        %dir,
        "world configured"
    );
    Ok((Some(net::Loader::spawn(Arc::new(world))?), info))
}

/// Opens the player save store and starts its writer thread (Шаг 18a).
///
/// Returns `None` when `save_dir` is unset: persistence stays off. Everything
/// happens before the port is bound, so a bad save root (a path inside the
/// world template or the vanilla stand, or an unwritable directory) fails the
/// start instead of surfacing on someone's join.
fn open_playerdata(
    config: &ServerConfig,
) -> Result<Option<(net::game::PlayerdataRuntime, playerdata::Writer)>, RunError> {
    let Some(save_dir) = config.save_dir.as_deref().map(str::trim) else {
        tracing::warn!(
            "no save_dir configured: player state is not saved between sessions \
             (see server.example.toml)"
        );
        return Ok(None);
    };
    // The schema every file is written with is the served save's: a mismatched
    // DataVersion would make the game reconsider the whole player document.
    let data_version = match config.world_dir.as_deref().map(str::trim) {
        Some(dir) => world::read_level_dat(Path::new(dir).join("level.dat"))?.data_version,
        None => 0,
    };
    let world_dir = config.world_dir.as_deref().map(str::trim);
    let vanilla_dir = config.vanilla_server_dir.as_deref().map(str::trim);
    let mut forbidden: Vec<(&'static str, &Path)> = Vec::new();
    if let Some(dir) = world_dir {
        forbidden.push(("world template", Path::new(dir)));
    }
    if let Some(dir) = vanilla_dir {
        forbidden.push(("vanilla stand", Path::new(dir)));
    }
    let store = Arc::new(playerdata::Store::open(save_dir, data_version, &forbidden)?);
    let (sink, writer) = playerdata::spawn((*store).clone(), playerdata::DEFAULT_CAPACITY)?;
    let runtime = net::game::PlayerdataRuntime::new(
        store,
        sink,
        config.force_gamemode,
        config.autosave_interval_ticks,
    );
    tracing::info!(
        save_dir,
        data_version,
        force_gamemode = config.force_gamemode,
        autosave_interval_ticks = config.autosave_interval_ticks,
        "player saves enabled"
    );
    Ok(Some((runtime, writer)))
}

/// The world template's identity as `world.json` records it (Шаг 22b).
///
/// `level.dat` is read twice on purpose: the SHA-256 pins the exact file the
/// diffs were written against, the `DataVersion` is the schema the chunk
/// palettes mean, and the world builder has to read the same file anyway.
fn world_identity(config: &ServerConfig) -> Result<worldsave::WorldIdentity, RunError> {
    let Some(dir) = config.world_dir.as_deref().map(str::trim) else {
        return Err(RunError::MissingSetting {
            setting: "world_dir",
            hint: WORLD_HINT,
        });
    };
    let level_dat = Path::new(dir).join("level.dat");
    let bytes = std::fs::read(&level_dat)?;
    let level = world::read_level_dat(&level_dat)?;
    let level_dat_sha256 = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(worldsave::WorldIdentity {
        template: dir.to_owned(),
        level_dat_sha256,
        data_version: level.data_version,
    })
}

/// The world-save runtime, its writer and the store the template reads its
/// diffs from (Шаг 22b).
struct WorldSaveHandles {
    /// The game thread's end of the writer queue.
    runtime: Arc<worldsave::Runtime>,
    /// The writer thread, drained after the game thread stops.
    writer: worldsave::Writer,
    /// The save root: `world.json`, `chunks/` and the diffs.
    store: Arc<worldsave::Store>,
}

/// Opens the world save and starts its writer thread (Шаг 22b).
///
/// Returns `None` when `save_dir` is unset: block changes stay in memory and
/// a restart starts from the template. Everything happens before the port is
/// bound, so a save that belongs to another template (another `level.dat`
/// SHA-256 or `DataVersion`) fails the start unless
/// `save_ignore_world_mismatch = true`, instead of silently serving a world
/// built from mismatched diffs.
fn open_worldsave(config: &ServerConfig) -> Result<Option<WorldSaveHandles>, RunError> {
    let Some(save_dir) = config.save_dir.as_deref().map(str::trim) else {
        tracing::warn!(
            "no save_dir configured: block changes are not saved between sessions \
             (see server.example.toml)"
        );
        return Ok(None);
    };
    let identity = world_identity(config)?;
    let store = worldsave::Store::open(save_dir, &identity, config.save_ignore_world_mismatch)?;
    let (runtime, writer) = worldsave::spawn(store.clone(), worldsave::DEFAULT_CAPACITY)?;
    tracing::info!(
        save_dir,
        template = %identity.template,
        data_version = identity.data_version,
        world_autosave_ticks = config.world_autosave_ticks,
        ignore_mismatch = config.save_ignore_world_mismatch,
        "world saves enabled"
    );
    Ok(Some(WorldSaveHandles {
        runtime: Arc::new(runtime),
        writer,
        store: Arc::new(store),
    }))
}

/// Binds the configured address and serves connections until an I/O error.
///
/// `game_data` holds the registries and tags loaded from the client jar
/// (see `data::load_from_jar`); the caller loads it once at startup so a
/// broken jar fails before the port is bound. Packet ids are resolved the
/// same way: a broken packets report fails before the port is bound.
pub async fn run(config: ServerConfig, game_data: Arc<GameData>) -> Result<(), RunError> {
    // One clear message instead of a server that starts and then serves an
    // empty world: the world template and the three reports come first.
    require_chunk_paths(&config)?;
    let packet_ids = load_packet_ids(&config)?;
    // Session recording (`MINECRAFT_SESSION_TRACE`) and the id → name
    // table every packet log line reads from, live before the first
    // connection arrives.
    net::trace::start();
    net::trace::install_names(net::trace::names_for(&packet_ids));
    // Player saves (Шаг 18a) and the world save (Шаг 22b) both start before
    // the port is bound, so a refused save root fails the start. The world
    // diffs are read by `build_world_with_save` below, before the warm-up.
    let (playerdata, playerdata_writer) = match open_playerdata(&config)? {
        Some((runtime, writer)) => (Some(Arc::new(runtime)), Some(writer)),
        None => (None, None),
    };
    let (worldsave, worldsave_writer, saved_store) = match open_worldsave(&config)? {
        Some(handles) => (
            Some(handles.runtime),
            Some(handles.writer),
            Some(handles.store),
        ),
        None => (None, None, None),
    };
    let shutdown_playerdata = playerdata.clone();
    let (loader, mut world) = build_world_with_save(&config, Arc::clone(&game_data), saved_store)?;
    let store = match &loader {
        Some(loader) => Some(warm_world(loader).await?),
        None => None,
    };
    if let Some(raw_override) = config.spawn_override.as_deref() {
        match config::parse_spawn_override(raw_override) {
            Ok(position)
                if store.as_ref().is_some_and(|store| {
                    store.position(position.x >> 4, position.z >> 4).is_some()
                }) =>
            {
                world.player_spawn_override = Some(position);
                tracing::info!(
                    x = position.x,
                    y = position.y,
                    z = position.z,
                    "development player spawn override enabled"
                );
            }
            Ok(position) => tracing::warn!(
                x = position.x,
                y = position.y,
                z = position.z,
                "development player spawn override chunk is not ready; using world spawn"
            ),
            Err(error) => {
                tracing::warn!(%error, "invalid development player spawn override; using world spawn")
            }
        }
    }
    // Development geometry (Шаг 15 п.2): off unless configured; applied to
    // the in-memory store before the port is bound, never written to the
    // world directory. The dirty map is then returned to the cells the save
    // had (Шаг 22b): the geometry is not saved, and neither is a stored diff
    // dropped.
    if let Some(path) = config.dev_geometry.as_deref() {
        apply_dev_geometry(path, store.as_deref())?;
    }
    let listener = TcpListener::bind(&config.listen).await?;
    let local = listener.local_addr()?;
    tracing::info!(
        %local,
        motd = %config.motd,
        max_players = config.max_players,
        protocol_version = config.protocol_version,
        version_name = %config.version_name,
        "listening"
    );
    let shutdown_store = store.clone();
    let game_config = net::game::GameConfig {
        workers: config.workers,
        region_side: config.region_side,
        tick_thread_priority: config.tick_thread_priority,
        store,
        hardness: load_hardness(&config)?,
        default_game_mode: config.game_mode,
        dig_tolerance_ticks: config.dig_tolerance_ticks,
        playerdata,
        worldsave: worldsave
            .as_ref()
            .map(|runtime| Arc::clone(runtime) as Arc<dyn net::game::WorldSave>),
        world_autosave_ticks: config.world_autosave_ticks,
        registries: load_registries(&config)?,
        ..net::game::GameConfig::default()
    };
    let serve_exit = net::serve_with_shutdown_status(
        game_config,
        listener,
        Arc::new(status_info(&config)),
        game_data,
        packet_ids,
        loader,
        world,
    )
    .await;
    if !serve_exit.game_stopped {
        // The detached game thread may still call set_block or stage a newer
        // player snapshot. Taking or closing either writer here would race it;
        // return a failing status and let process exit terminate the threads.
        return Err(RunError::GameShutdown);
    }
    let serve_result = serve_exit.result;
    let mut shutdown_error = None;
    let mut world_flush_ok = true;
    // The game thread has stopped changing chunks. Queue one complete final
    // snapshot after every periodic batch and wait for its actual write
    // result; this runs even when the accept loop returned an I/O error.
    // The world flush is awaited before the player writer is drained
    // (docs/WORLD-SAVE.md §6, Шаг 22b).
    if let (Some(runtime), Some(store)) = (worldsave.as_deref(), shutdown_store.as_deref()) {
        for done in runtime.completed() {
            store.saved(done);
        }
        let final_batch = store.shutdown_batch();
        let chunks = final_batch.len();
        match runtime.flush(final_batch, std::time::Duration::from_secs(5)) {
            Ok(()) => tracing::info!(chunks, "final world save flushed"),
            Err(error) => {
                tracing::warn!(%error, chunks, "final world save was not flushed completely");
                world_flush_ok = false;
                shutdown_error = Some(RunError::Worldsave(error));
            }
        }
    }
    if world_flush_ok {
        if let Some(runtime) = shutdown_playerdata.as_deref() {
            let (queued, dropped) = runtime.flush_final();
            if dropped == 0 {
                tracing::info!(
                    players = queued,
                    "final player saves queued after the world flush"
                );
            } else {
                tracing::warn!(queued, dropped, "the final player-save queue was full");
                shutdown_error = Some(RunError::FinalPlayerQueue(dropped));
            }
        }
    } else if shutdown_playerdata.is_some() {
        tracing::warn!(
            "final player snapshots were not queued because the final world flush failed"
        );
    }
    // Drain the writer threads even after a failed final write. A timeout is
    // a warning: the listener and game thread are already stopped.
    if let Some(writer) = worldsave_writer {
        match writer.shutdown(std::time::Duration::from_secs(5)) {
            Ok(()) => tracing::info!("world-save writer stopped"),
            Err(error) => {
                tracing::warn!(%error, "the world writer did not stop cleanly");
                shutdown_error.get_or_insert(RunError::Worldsave(error));
            }
        }
    }
    if let Some(writer) = playerdata_writer {
        match writer.shutdown(std::time::Duration::from_secs(5)) {
            Ok(()) => tracing::info!("player saves flushed"),
            Err(error) => {
                tracing::warn!(%error, "the player writer did not stop cleanly");
                shutdown_error.get_or_insert(RunError::Playerdata(error));
            }
        }
    }
    if let Some(error) = shutdown_error {
        if let Err(serve_error) = serve_result {
            tracing::warn!(%serve_error, "the accept loop also failed during shutdown");
        }
        return Err(error);
    }
    serve_result?;
    Ok(())
}

/// Applies the configured development geometry file to the in-memory store.
///
/// Warns once when enabled (the flag is development-only), then reads,
/// resolves and writes each command. A missing store (a world that failed to
/// warm up) skips the step with a warning rather than failing a server that
/// has nothing to write into. Any bad line aborts startup with its number.
fn apply_dev_geometry(path: &str, store: Option<&net::ChunkStore>) -> Result<(), RunError> {
    tracing::warn!(
        path,
        "development geometry enabled: applying setblock/fill in memory only \
         (the world directory is not written)"
    );
    let Some(store) = store else {
        tracing::warn!("no world store is ready; development geometry skipped");
        return Ok(());
    };
    let touched = geometry::apply_file(store, store.blocks(), path)?;
    // The geometry is development-only and must not reach the save: return the
    // dirty map to the cells the save had (Шаг 22b).
    store.reset_dirty();
    tracing::info!(
        path,
        blocks = touched,
        "development geometry applied to the in-memory store"
    );
    Ok(())
}

/// Reads the world batch once before the port is bound, so the start log
/// shows how many chunks are ready to be sent (the live check asks for
/// exactly that number) and the game thread gets the very store the
/// connections send from (Шаг 13b) — the one object a later block write and
/// a later join both see. A failure is fatal before `bind`: in particular an
/// unreadable save directory must not silently turn a persisted world into
/// the untouched template for every joining player (Шаг 22b).
async fn warm_world(loader: &net::Loader) -> Result<Arc<net::ChunkStore>, RunError> {
    let reply = loader
        .load_batch()
        .await
        .map_err(|error| RunError::Warmup(error.to_string()))?;
    match reply.await {
        Ok(Ok(store)) => {
            tracing::info!(
                ready_chunks = store.len(),
                "world warm-up finished: chunks ready to send"
            );
            Ok(store)
        }
        Ok(Err(error)) => Err(RunError::Warmup(error.to_string())),
        Err(error) => Err(RunError::Warmup(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::StatusResponse;

    /// Test config: the fields the id loader does not read may be anything.
    fn config(report: Option<String>) -> ServerConfig {
        ServerConfig {
            listen: "127.0.0.1:0".to_owned(),
            motd: "test".to_owned(),
            max_players: 1,
            protocol_version: 1,
            version_name: "test".to_owned(),
            client_jar: "client.jar".to_owned(),
            packets_report: report,
            registries_report: None,
            blocks_report: None,
            collision_report: None,
            hardness_report: None,
            dig_tolerance_ticks: None,
            world_dir: None,
            vanilla_server_jar: None,
            vanilla_server_dir: None,
            view_distance: config::DEFAULT_DISTANCES,
            simulation_distance: config::DEFAULT_DISTANCES,
            workers: 0,
            region_side: 0,
            tick_thread_priority: false,
            game_mode: 0,
            spawn_override: None,
            dev_geometry: None,
            save_dir: None,
            force_gamemode: true,
            autosave_interval_ticks: config::DEFAULT_AUTOSAVE_TICKS,
            world_autosave_ticks: config::DEFAULT_AUTOSAVE_TICKS,
            save_ignore_world_mismatch: false,
        }
    }

    /// Captures the `tracing` lines emitted on this thread while `f` runs, so a
    /// test can assert on a startup warning without a live server log. Only the
    /// calling thread's events are seen: `with_default` installs a thread-local
    /// subscriber, which is exactly what `load_collisions`/`build_world` log
    /// from.
    fn capture_logs(f: impl FnOnce()) -> String {
        use std::io::Write;
        use std::sync::{Arc, Mutex};

        struct Sink(Arc<Mutex<Vec<u8>>>);
        impl Write for Sink {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                self.0.lock().expect("log sink").extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer = {
            let buffer = Arc::clone(&buffer);
            move || Sink(Arc::clone(&buffer))
        };
        let subscriber = tracing_subscriber::fmt()
            .with_writer(writer)
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::with_default(subscriber, f);
        let bytes = buffer.lock().expect("log sink").clone();
        String::from_utf8(bytes).expect("utf-8 log lines")
    }

    #[test]
    fn without_a_report_the_compiled_ids_are_used() {
        let ids = load_packet_ids(&config(None)).expect("compiled ids never fail");
        assert!(ids.is_empty());
        assert_eq!(ids.id(StatusResponse::KEY), StatusResponse::ID);
    }

    #[test]
    fn a_report_provides_the_ids() {
        // Fictional override: proof that the id comes from the report.
        let json = data::fixtures::packets_report_json(&[(StatusResponse::KEY, 0x7F)]);
        let path = std::env::temp_dir().join("minerust-packets-report-test.json");
        std::fs::write(&path, &json).expect("write fixture report");
        let ids = load_packet_ids(&config(Some(path.display().to_string()))).expect("valid report");
        assert_eq!(ids.id(StatusResponse::KEY), 0x7F);
    }

    #[test]
    fn a_broken_report_is_an_error_before_binding() {
        let error = load_packet_ids(&config(Some("definitely-not-here.json".to_owned())))
            .expect_err("missing report");
        assert!(matches!(error, RunError::Data(_)));
    }

    #[test]
    fn a_report_with_a_missing_packet_is_rejected() {
        // `minecraft:login` sorts into the middle of the play/clientbound
        // entries, so the entry with its leading comma can be dropped and
        // the remaining report stays valid JSON.
        let key = protocol::LoginPlay::KEY;
        let json = data::fixtures::packets_report_json(&[]);
        let text = json.replace(
            &format!(",\"{}\":{{\"protocol_id\":{}}}", key.name, key.default_id),
            "",
        );
        assert_ne!(text.len(), json.len(), "the key must have been removed");
        let path = std::env::temp_dir().join("minerust-packets-report-incomplete.json");
        std::fs::write(&path, &text).expect("write fixture report");
        let error = load_packet_ids(&config(Some(path.display().to_string())))
            .expect_err("incomplete report");
        assert!(matches!(error, RunError::Data(_)), "{error}");
    }

    /// A gzipped disk NBT document of two nested compounds: `Data` with its
    /// `DataVersion` int and the `spawn` compound holding the `pos` array.
    fn level_document(data_version: i32, spawn: [i32; 3]) -> Vec<u8> {
        let mut doc = vec![0x0A, 0x00, 0x00]; // root compound, empty name
        doc.extend_from_slice(&[0x0A, 0x00, 0x04]); // compound `Data`
        doc.extend_from_slice(b"Data");
        doc.extend_from_slice(&[0x03, 0x00, 0x0B]); // int `DataVersion`
        doc.extend_from_slice(b"DataVersion");
        doc.extend_from_slice(&data_version.to_be_bytes());
        doc.extend_from_slice(&[0x0A, 0x00, 0x05]); // compound `spawn`
        doc.extend_from_slice(b"spawn");
        doc.extend_from_slice(&[0x0B, 0x00, 0x03]); // int array `pos`
        doc.extend_from_slice(b"pos");
        doc.extend_from_slice(&3_i32.to_be_bytes());
        for coordinate in spawn {
            doc.extend_from_slice(&coordinate.to_be_bytes());
        }
        doc.extend_from_slice(&[0x00, 0x00, 0x00]); // spawn, Data, root
        gzip(&doc)
    }

    /// A gzipped disk NBT document in the shape of the real template: the
    /// root holds the `data` compound (with `bonus_chest`, the `seed`
    /// long, `generate_structures` and `dimensions`) and the `DataVersion`
    /// int — walked byte by byte on the owner's save (the live check of
    /// step 3.3 failed on a seed missing from this envelope).
    fn settings_document(seed: i64) -> Vec<u8> {
        let mut doc = vec![0x0A, 0x00, 0x00]; // root compound, empty name
        doc.extend_from_slice(&[0x0A, 0x00, 0x04]); // compound `data`
        doc.extend_from_slice(b"data");
        doc.extend_from_slice(&[0x01, 0x00, 0x0B]); // byte `bonus_chest`
        doc.extend_from_slice(b"bonus_chest");
        doc.push(0x00); // false
        doc.extend_from_slice(&[0x04, 0x00, 0x04]); // long `seed`
        doc.extend_from_slice(b"seed");
        doc.extend_from_slice(&seed.to_be_bytes());
        doc.extend_from_slice(&[0x01, 0x00, 0x13]); // byte `generate_structures`
        doc.extend_from_slice(b"generate_structures");
        doc.push(0x01); // true
        doc.extend_from_slice(&[0x0A, 0x00, 0x0A]); // compound `dimensions`
        doc.extend_from_slice(b"dimensions");
        doc.push(0x00); // empty map
        doc.push(0x00); // end of data
        doc.extend_from_slice(&[0x03, 0x00, 0x0B]); // int `DataVersion`
        doc.extend_from_slice(b"DataVersion");
        doc.extend_from_slice(&5023_i32.to_be_bytes());
        doc.push(0x00); // root end
        gzip(&doc)
    }

    /// Gzip container around `document` (the shape a save stores in).
    fn gzip(document: &[u8]) -> Vec<u8> {
        use std::io::Write as _;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(document).expect("gzip stream");
        encoder.finish().expect("gzip finished")
    }

    /// A minimal save: both world documents and — with `with_region` — the
    /// region directory `build_world` demands. Returns the world config and
    /// its root (for the caller to delete afterwards).
    fn temp_world(name: &str, with_region: bool) -> (ServerConfig, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("minerust-{name}"));
        let data = root.join("data/minecraft");
        std::fs::create_dir_all(&data).expect("data dir");
        if with_region {
            std::fs::create_dir_all(root.join("dimensions/minecraft/overworld/region"))
                .expect("region dir");
        }
        std::fs::write(root.join("level.dat"), level_document(5023, [96, 145, -32]))
            .expect("level.dat");
        std::fs::write(
            data.join("world_gen_settings.dat"),
            settings_document(12345),
        )
        .expect("world_gen_settings.dat");
        let mut config = config(None);
        config.world_dir = Some(root.display().to_string());
        (config, root)
    }

    /// Step 3.3e: the join values come out of the save — spawn block from
    /// `level.dat`, hashed seed from `world_gen_settings.dat`.
    #[test]
    fn build_world_reads_the_saved_spawn_and_seed() {
        let (config, root) = temp_world("build-world", true);
        let (loader, info) =
            build_world(&config, Arc::new(GameData::default())).expect("valid world");
        assert!(loader.is_some(), "the pool is built for a world");
        assert_eq!(
            info,
            net::WorldInfo {
                spawn: protocol::Position {
                    x: 96,
                    y: 145,
                    z: -32
                },
                player_spawn_override: None,
                hashed_seed: protocol::hashed_seed(12345),
                view_distance: config.view_distance,
                simulation_distance: config.simulation_distance,
            },
            "spawn and seed from the save, distances from the config"
        );
        drop(loader);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Шаг 16.0a: a server started **without** `collision_report` must still
    /// work — the forced crouch/crawl rule is simply off — and the log must say
    /// so. The settings branch is covered directly, and `build_world` proves the
    /// whole startup path accepts the missing report.
    #[test]
    fn a_server_without_the_collision_report_disables_the_crawl_rule_and_warns() {
        // The setting: absent means an empty table, and the store treats an
        // empty table as "the rule cannot be evaluated" — `free_headroom`
        // returns `None` and the forced pose stays off (net's
        // `the_free_headroom_reads_the_collision_report` asserts the `None`;
        // `Forced::from_headroom(None)` is `Forced::None`).
        let config = config(None);
        let warning = capture_logs(|| {
            let collisions = load_collisions(&config).expect("a missing report is not an error");
            assert!(collisions.is_empty(), "no report, no shapes");
        });
        assert!(
            warning.contains("no collision_report"),
            "the warning names the missing setting: {warning}"
        );
        assert!(
            warning.contains("crouch/crawl"),
            "the warning names the disabled rule: {warning}"
        );
        assert!(
            warning.contains("collision_report=<path>") && warning.contains("`shapes`"),
            "the single warning carries the hint (setting and generator command): {warning}"
        );
        assert_eq!(
            warning.matches("no collision_report").count(),
            1,
            "the hint is logged once, not per chunk or per tick: {warning}"
        );

        // The whole startup path takes the missing report: a real (minimal) save
        // still builds its world and loader pool, with the warning on the way.
        let (config, root) = temp_world("build-world-no-collision", true);
        let startup = capture_logs(|| {
            let (loader, _info) =
                build_world(&config, Arc::new(GameData::default())).expect("valid world");
            assert!(loader.is_some(), "the server still builds its chunk pool");
            drop(loader);
        });
        assert!(
            startup.contains("no collision_report"),
            "the startup path warns as well: {startup}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Шаг 16a.1: a server started **without** `hardness_report` must still work
    /// — the mining timer keeps trusting the client — and the log must say so.
    /// A configured report is loaded and validated instead.
    #[test]
    fn a_server_without_the_hardness_report_keeps_trusting_the_client_and_warns() {
        let bare = config(None);
        let warning = capture_logs(|| {
            let hardness = load_hardness(&bare).expect("a missing report is not an error");
            assert!(hardness.is_none(), "no report, no hardness table");
        });
        assert!(
            warning.contains("no hardness_report"),
            "the warning names the missing setting: {warning}"
        );
        assert!(
            warning.contains("crack stages"),
            "the warning names what stays off: {warning}"
        );
        assert!(
            warning.contains("hardness_report=<path>") && warning.contains("`hardness`"),
            "the single warning carries the hint (setting and generator command): {warning}"
        );
        assert_eq!(
            warning.matches("no hardness_report").count(),
            1,
            "the hint is logged once, not per dig: {warning}"
        );

        // A configured report is read and parsed: the wrong path fails startup.
        let missing = ServerConfig {
            hardness_report: Some("no-such-hardness.json".to_owned()),
            ..config(None)
        };
        assert!(
            matches!(
                load_hardness(&missing),
                Err(RunError::Data(data::DataError::HardnessReportRead { .. }))
            ),
            "a broken path fails the start, like the collision report"
        );
    }

    /// The startup refuses a config without the chunk paths, naming the
    /// first missing field — never a server that runs and sends nothing
    /// (the bug the live check of step 3.3 ran into).
    #[test]
    fn a_startup_without_the_chunk_paths_is_refused_by_name() {
        let mut config = config(None);

        let error = require_chunk_paths(&config).expect_err("world_dir missing");
        match &error {
            RunError::MissingSetting { setting, .. } => assert_eq!(*setting, "world_dir"),
            other => panic!("unexpected error: {other}"),
        }
        assert!(error.to_string().contains("server.toml"), "{error}");

        config.world_dir = Some("world".to_owned());
        let error = require_chunk_paths(&config).expect_err("blocks_report missing");
        assert!(
            matches!(
                &error,
                RunError::MissingSetting {
                    setting: "blocks_report",
                    ..
                }
            ),
            "{error}"
        );

        config.blocks_report = Some("blocks.json".to_owned());
        let error = require_chunk_paths(&config).expect_err("registries_report missing");
        assert!(
            matches!(
                &error,
                RunError::MissingSetting {
                    setting: "registries_report",
                    ..
                }
            ),
            "{error}"
        );

        config.registries_report = Some("registries.json".to_owned());
        let error = require_chunk_paths(&config).expect_err("packets_report missing");
        assert!(
            matches!(
                &error,
                RunError::MissingSetting {
                    setting: "packets_report",
                    ..
                }
            ),
            "{error}"
        );

        config.packets_report = Some("packets.json".to_owned());
        require_chunk_paths(&config).expect("all four chunk paths are set");

        // A blank path is as missing as no path at all.
        config.world_dir = Some("  ".to_owned());
        require_chunk_paths(&config).expect_err("blank world_dir");
    }

    /// The library-level behaviour without `world_dir` (used only below
    /// `require_chunk_paths`, e.g. by the parity self-session): no pool,
    /// the default spawn and no hashed seed — the behaviour parity records.
    #[test]
    fn without_a_world_dir_the_empty_world_defaults_are_used() {
        let (loader, info) =
            build_world(&config(None), Arc::new(GameData::default())).expect("no world");
        assert!(loader.is_none(), "no pool without a world");
        assert_eq!(info, net::WorldInfo::default(), "same as before 3.3e");
    }

    /// A save without its region directory is refused at startup, not on
    /// somebody's join.
    #[test]
    fn a_world_without_a_region_directory_is_refused() {
        let (config, root) = temp_world("build-world-noregion", false);
        // The pool itself is not `Debug`, so match instead of `expect_err`.
        let error = match build_world(&config, Arc::new(GameData::default())) {
            Ok(_) => panic!("a save without regions must be refused"),
            Err(error) => error,
        };
        assert!(matches!(error, RunError::InvalidWorld(_)), "{error}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
