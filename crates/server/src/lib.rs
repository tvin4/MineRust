//! Server library: configuration loading and the startup sequence.
//!
//! The binary in `main.rs` is a thin wrapper around [`run`]. Keeping the
//! logic in a library allows the integration tests to start a real server on
//! an ephemeral port without spawning a subprocess.

mod config;
mod template;

pub use config::{load_config, ConfigError, ServerConfig};

use std::path::Path;
use std::sync::Arc;

use data::{BlockStates, GameData};
use protocol::StatusInfo;
use tokio::net::TcpListener;

use template::{TemplateWorld, WireTables};

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

/// Builds the chunk loader pool and the join values (`net::WorldInfo`) of
/// the configured world, or the empty defaults for no `world_dir` (the M2
/// behaviour: no chunks, spawn at eight and no seed).
///
/// Everything happens before the port is bound: a world whose `level.dat`
/// or `world_gen_settings.dat` cannot be read or whose region directory is
/// missing fails at startup, never as a failed read on someone's join.
fn build_world(
    config: &ServerConfig,
    game_data: Arc<GameData>,
) -> Result<(Option<net::Loader>, net::WorldInfo), RunError> {
    let Some(dir) = config.world_dir.as_deref().map(str::trim) else {
        tracing::info!("no world_dir configured: the server sends no chunks");
        return Ok((None, net::WorldInfo::default()));
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
    let info = net::WorldInfo {
        spawn: level.spawn,
        hashed_seed: protocol::hashed_seed(settings.seed),
    };
    let blocks = match config.blocks_report.as_deref() {
        Some(path) => BlockStates::load(path)?,
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
    let tables = Arc::new(WireTables::new(blocks, registries, game_data));
    let world = TemplateWorld::new(region_dir, level.data_version, tables);
    tracing::info!(
        data_version = level.data_version,
        seed = settings.seed,
        hashed_seed = info.hashed_seed,
        spawn_x = info.spawn.x,
        spawn_y = info.spawn.y,
        spawn_z = info.spawn.z,
        %dir,
        "world configured"
    );
    Ok((Some(net::Loader::spawn(Arc::new(world))?), info))
}

/// Binds the configured address and serves connections until an I/O error.
///
/// `game_data` holds the registries and tags loaded from the client jar
/// (see `data::load_from_jar`); the caller loads it once at startup so a
/// broken jar fails before the port is bound. Packet ids are resolved the
/// same way: a broken packets report fails before the port is bound.
pub async fn run(config: ServerConfig, game_data: Arc<GameData>) -> Result<(), RunError> {
    let packet_ids = load_packet_ids(&config)?;
    let (loader, world) = build_world(&config, Arc::clone(&game_data))?;
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
    net::serve(
        listener,
        Arc::new(status_info(&config)),
        game_data,
        packet_ids,
        loader,
        world,
    )
    .await?;
    Ok(())
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
            world_dir: None,
            vanilla_server_jar: None,
            vanilla_server_dir: None,
        }
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

    /// A gzipped disk NBT document: the root's `seed` long.
    fn settings_document(seed: i64) -> Vec<u8> {
        let mut doc = vec![0x0A, 0x00, 0x00]; // root compound, empty name
        doc.extend_from_slice(&[0x04, 0x00, 0x04]); // long `seed`
        doc.extend_from_slice(b"seed");
        doc.extend_from_slice(&seed.to_be_bytes());
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
                hashed_seed: protocol::hashed_seed(12345),
            },
            "spawn from level.dat, seed hashed the verified way"
        );
        drop(loader);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Without `world_dir` the server stays the M2 one: no pool, the
    /// default spawn and no hashed seed — the behaviour parity records.
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
