//! Step 5 of the live check: our own server, the real saved world, the
//! scripted client — a recorded session that live_vs_vanilla reads.
//!
//! Unlike the join slice (which serves no chunks at all), this one starts
//! the in-process server on the configured `world_dir`, so the client
//! receives what a real player would: the join sequence, the chunk cache
//! centre of the spawn and the chunks under it. The recording lands in
//! `target/parity/self-session.json` and is asserted against the live
//! check's expectations:
//!
//! * the centre of the chunk cache is the spawn in chunks — (6, -2);
//! * exactly nine `level_chunk_with_light` arrive (the ready chunks of the
//!   frozen template in every recorded run);
//! * the chunk (6, -2) — the one under the join position (96.5, 145,
//!   -31.5) — is among them;
//! * no chunk falls outside centre ± view distance (10).
//!
//! Manual-only like every parity test:
//!
//! ```text
//! cargo test -p parity --test self_session -- --ignored --nocapture
//! ```
//!
//! Without a configured `world_dir` the test prints a SKIP instead of
//! pretending an empty world is the recorded behaviour.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parity::{join_steps, load_parity_env, Client, ParityEnv, Step, Trace, TraceDirection};
use protocol::LevelChunkWithLight;

/// The chunk a join at (96.5, 145, -31.5) must be served under.
const EXPECTED_CHUNK: (i32, i32) = (6, -2);

/// Ready chunks of the frozen template in every recorded run.
const EXPECTED_CHUNKS: usize = 9;

/// The chunk cache radius the join announces (net's VIEW_DISTANCE).
// SOURCE: our own join constant, mirrored by live_vs_vanilla.
const VIEW_DISTANCE: i32 = 10;

/// Where the recording lands: `<workspace>/target/parity/`.
fn out_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("parity")
}

/// Builds the world out of the configured template, saying what broke
/// when it does (the seed-envelope blocker of the live check was fixed by
/// reading `data.seed`; a remaining error is a new one to look at).
fn build_or_explain(
    env: &ParityEnv,
    game_data: Arc<data::GameData>,
) -> (Option<net::Loader>, net::WorldInfo) {
    server::build_world(&env.config, game_data)
        .unwrap_or_else(|error| panic!("the saved world cannot be built: {error}"))
}

/// Starts our own server in-process with the loaded world on an ephemeral
/// port. The in-process server is aborted by the guard on every exit path.
async fn start_with_world(
    env: &ParityEnv,
    ids: Arc<data::PacketIds>,
) -> (SocketAddr, tokio::task::AbortHandle) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr = listener.local_addr().expect("local addr");
    let status = Arc::new(server::status_info(&env.config));
    let game_data = Arc::new(
        data::load_from_jar(
            &env.config.client_jar,
            env.config.registries_report.as_deref().map(Path::new),
        )
        .expect("game data from the jar"),
    );
    let (loader, world) = build_or_explain(env, Arc::clone(&game_data));
    let server = tokio::spawn(net::serve(listener, status, game_data, ids, loader, world));
    (addr, server.abort_handle())
}

/// Aborts the in-process server on every exit path, including a panic.
struct ServerGuard {
    /// Handle of the spawned `net::serve` task.
    abort: Option<tokio::task::AbortHandle>,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(abort) = self.abort.take() {
            abort.abort();
        }
    }
}

/// Runs the join script and keeps the recording.
async fn record_self_session(
    env: &ParityEnv,
    ids: Arc<data::PacketIds>,
    addr: SocketAddr,
) -> Trace {
    let mut steps = join_steps(env, &ids, addr.port());
    // The join sequence plus the ready chunks, then two seconds of silence
    // to catch everything the server wants to say right after the batch.
    steps.push(Step::quiet(Duration::from_secs(2), Duration::from_secs(60)));
    let mut client = Client::connect("self", addr, Arc::clone(&ids))
        .await
        .expect("connect to our own server");
    if let Err(error) = client.run(&steps).await {
        let partial = client.into_trace();
        partial.save(&out_dir().join("self-session.json")).ok();
        panic!("script failed after {} packets: {error}", partial.len());
    }
    let trace = client.into_trace();
    trace
        .save(&out_dir().join("self-session.json"))
        .expect("trace must be written");
    trace
}

/// Payload bytes of one chunk packet; the coordinate prefix is the first
/// two big-endian i32 the encode writes (see protocol's LevelChunkWithLight).
fn chunk_coordinates(trace: &Trace, ids: &data::PacketIds) -> Vec<(i32, i32)> {
    let chunk_id = ids.id(LevelChunkWithLight::KEY);
    trace
        .packets
        .iter()
        .filter(|entry| entry.id == chunk_id && entry.direction == TraceDirection::Clientbound)
        .filter_map(|entry| {
            let bytes = parity::from_hex(&entry.payload).ok()?;
            let (x, rest) = bytes.split_first_chunk::<4>()?;
            let (z, _) = rest.split_first_chunk::<4>()?;
            Some((i32::from_be_bytes(*x), i32::from_be_bytes(*z)))
        })
        .collect()
}

/// The chunk cache centre of the recording: the payload of the
/// `set_chunk_cache_center` join packet holds two VarInts.
fn chunk_center(trace: &Trace, ids: &data::PacketIds) -> Option<(i32, i32)> {
    let center_id = ids.id(protocol::SetCenterChunk::KEY);
    trace
        .packets
        .iter()
        .find(|entry| entry.id == center_id && entry.direction == TraceDirection::Clientbound)
        .and_then(|entry| {
            let bytes = parity::from_hex(&entry.payload).ok()?;
            let (x, used) = protocol::decode_varint(&bytes).ok()?;
            let (z, _) = protocol::decode_varint(&bytes[used..]).ok()?;
            Some((x, z))
        })
}

#[tokio::test]
#[ignore = "requires the owner's world template (C:\\MineRust-data)"]
async fn our_server_serves_the_world_of_the_live_check() {
    let env = match load_parity_env() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            return;
        }
    };
    if env
        .config
        .world_dir
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
    {
        eprintln!("SKIP: no world_dir in server.toml — a server without a world sends no chunks");
        return;
    }
    let ids = Arc::new(env.packet_ids().expect("packet ids"));

    let mut guard = ServerGuard { abort: None };
    let (addr, abort) = start_with_world(&env, Arc::clone(&ids)).await;
    guard.abort = Some(abort);
    let trace = record_self_session(&env, Arc::clone(&ids), addr).await;
    drop(guard);

    // -- The four expectations of step 5 ----------------------------------
    assert!(
        !trace.is_empty(),
        "no packets recorded — the server never answered"
    );

    let center = chunk_center(&trace, &ids);
    assert_eq!(
        center,
        Some(EXPECTED_CHUNK),
        "the chunk cache centre must be the spawn (96, 145, -32) in chunks"
    );

    let coords = chunk_coordinates(&trace, &ids);
    assert_eq!(
        coords.len(),
        EXPECTED_CHUNKS,
        "the frozen template sends nine ready chunks; got: {coords:?}"
    );
    assert!(
        coords.contains(&EXPECTED_CHUNK),
        "chunk {EXPECTED_CHUNK:?} under the join position (96.5, 145, -31.5) must arrive; got: {coords:?}"
    );
    if let Some((cx, cz)) = center {
        let outside: Vec<(i32, i32)> = coords
            .iter()
            .copied()
            .filter(|(x, z)| (x - cx).abs() > VIEW_DISTANCE || (z - cz).abs() > VIEW_DISTANCE)
            .collect();
        assert!(
            outside.is_empty(),
            "chunks outside centre ±{VIEW_DISTANCE}: {outside:?}"
        );
    }

    eprintln!(
        "self-session written: {} packets, center {center:?}, chunks {coords:?}",
        trace.len()
    );
}
