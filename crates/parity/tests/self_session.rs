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
//! * the batch of `level_chunk_with_light` opens with `chunk_batch_start`
//!   and carries every ready chunk of the template inside the send radius
//!   (289 = the 8-chunk square of the scripted client's own view
//!   distance, part A 6);
//! * the chunk (6, -2) — the one under the join position (96.5, 145,
//!   -31.5) — is among them;
//! * no chunk falls outside centre ± send radius (8).
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
use std::sync::{Arc, Mutex};
use std::time::Duration;

use parity::{join_steps, load_parity_env, Client, ParityEnv, Step, Trace, TraceDirection};
use protocol::LevelChunkWithLight;
use tracing_subscriber::util::SubscriberInitExt;

/// The chunk a join at (96.5, 145, -31.5) must be served under.
const EXPECTED_CHUNK: (i32, i32) = (6, -2);

/// Chunks of template v2 inside the send radius: the client asks for 8
/// (`join_steps`), the server announces 10 (`server.toml`,
/// `view_distance`, defaulting to `server::DEFAULT_DISTANCES`), the
/// template has 729 ready chunks — `min(8, 10)` gives the 17×17 square,
/// and every chunk of that square is ready (`full`/`initialize_light`).
/// Counted by `target/parity/chunk_status.py compare` (part A, 6).
const EXPECTED_CHUNKS: usize = 289;

/// The radius this session sends in: the scripted client's own request
/// (`join_steps` sends `view_distance: 8`), the server caps it at 10.
// SOURCE: our own rules — `join_steps` and `net::connection::play::send_radius`.
const SEND_RADIUS: i32 = 8;

/// How long the burst may take to *start*: the loader reads the whole
/// save on a pool thread (thousands of stored chunks, debug build), which
/// is seconds, never the two seconds of the old quiet drain.
const BATCH_WAIT: Duration = Duration::from_secs(90);

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
    // 1. everything up to the opening of the chunk burst: the join packets
    //    arrive at once, the burst only when the loader pool has built the
    //    payloads of the whole save — a quiet drain that short would end
    //    before the first chunk and record nothing (part A).
    steps.push(Step::wait(
        ids.id(protocol::ChunkBatchStart::KEY),
        BATCH_WAIT,
    ));
    // 2. the burst itself, then two seconds of silence to catch everything
    //    the server wants to say right after it.
    steps.push(Step::quiet(Duration::from_secs(2), Duration::from_secs(60)));
    let mut client = Client::connect("self", addr, Arc::clone(&ids))
        .await
        .expect("connect to our own server");
    // A script that stops early (a batch that never starts) keeps its
    // partial recording: the assertions below report what really arrived
    // instead of hiding it behind one generic client error.
    let outcome = client.run(&steps).await;
    report_chunk_arrivals(
        "first join",
        client.arrivals(),
        ids.id(LevelChunkWithLight::KEY),
    );
    let trace = client.into_trace();
    trace
        .save(&out_dir().join("self-session.json"))
        .expect("trace must be written");
    if let Err(error) = outcome {
        if trace.is_empty() {
            panic!("the server never answered: {error}");
        }
        eprintln!("script stopped early: {error}");
    }
    trace
}

/// The part A 2 measurement of one connection: the span from the connect
/// attempt to the first and to the last `level_chunk_with_light` on the
/// wire, printed as `MEASURE ...`. A parity test never asserts these — it
/// must stay green while the number itself improves — but the owner asked
/// for before/after figures, so both runs of this test print the same
/// line to compare against (release build: ignored, `--nocapture`).
fn report_chunk_arrivals(tag: &str, arrivals: &[(u32, Duration)], chunk_id: u32) {
    let at: Vec<Duration> = arrivals
        .iter()
        .filter(|(id, _)| *id == chunk_id)
        .map(|(_, elapsed)| *elapsed)
        .collect();
    match (at.first(), at.last()) {
        (Some(first), Some(last)) => eprintln!(
            "MEASURE {tag}: connect -> first chunk {} ms, connect -> last chunk {} ms ({} chunks)",
            first.as_millis(),
            last.as_millis(),
            at.len()
        ),
        _ => eprintln!("MEASURE {tag}: no chunk arrived"),
    }
}

/// A second join of the same world inside one process: the measurement
/// the world batch cache exists for. The first join reads the save (that
/// read is what still shows in its own span above); every join after it
/// — including this one — must be served from the shared immutable batch
/// without touching the region files again.
async fn measure_second_join(env: &ParityEnv, ids: Arc<data::PacketIds>, addr: SocketAddr) {
    let mut steps = join_steps(env, &ids, addr.port());
    steps.push(Step::wait(
        ids.id(protocol::ChunkBatchStart::KEY),
        BATCH_WAIT,
    ));
    steps.push(Step::quiet(Duration::from_secs(2), Duration::from_secs(60)));
    let mut client = Client::connect("self-second", addr, Arc::clone(&ids))
        .await
        .expect("second connect to our own server");
    let _ = client.run(&steps).await; // only the timing is of interest here
    report_chunk_arrivals(
        "second join",
        client.arrivals(),
        ids.id(LevelChunkWithLight::KEY),
    );
}

/// Everything the subscriber of the in-process server printed during the
/// two joins, kept as plain text so the end of the test can count it.
struct LogSink(Arc<Mutex<String>>);

impl std::io::Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if let Ok(text) = std::str::from_utf8(buf) {
            self.0.lock().expect("log sink uncontended").push_str(text);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Part A 2/3: what a join costs the log at info level. The subscriber
/// lives for the whole test and stops every `debug!` at its level filter,
/// so the printed figures are exactly what a default (`info`) server run
/// would write to its console — the per-chunk lines of one burst in
/// particular (289 chunks x two joins). Like every `MEASURE` line of this
/// test it is printed, never asserted.
fn count_info_lines(logged: &str) {
    let lines: Vec<&str> = logged.lines().filter(|line| !line.is_empty()).collect();
    let per_chunk = lines
        .iter()
        .filter(|line| line.contains("level_chunk_with_light sent"))
        .count();
    eprintln!(
        "MEASURE logs: {n} info lines over two joins, {per_chunk} of them per-chunk",
        n = lines.len()
    );
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

    // Part A 2/3: capture every info line of the in-process server for the
    // MEASURE line at the end (the guard keeps the subscriber installed).
    let logged = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&logged);
    let _log_guard = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(move || LogSink(Arc::clone(&sink)))
        .finish()
        .set_default();

    let mut guard = ServerGuard { abort: None };
    let (addr, abort) = start_with_world(&env, Arc::clone(&ids)).await;
    guard.abort = Some(abort);
    let trace = record_self_session(&env, Arc::clone(&ids), addr).await;
    // Part A 2: the same join once more, in the same process, so the cache
    // (or the absence of one) is what the second MEASURE line compares.
    measure_second_join(&env, Arc::clone(&ids), addr).await;
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
        "template v2 must send the {} ready chunks of radius {SEND_RADIUS}; got {} — {:?}",
        EXPECTED_CHUNKS,
        coords.len(),
        coords.iter().take(12).collect::<Vec<_>>()
    );
    assert!(
        coords.contains(&EXPECTED_CHUNK),
        "chunk {EXPECTED_CHUNK:?} under the join position (96.5, 145, -31.5) must arrive; got {} chunks",
        coords.len()
    );
    if let Some((cx, cz)) = center {
        let outside: Vec<(i32, i32)> = coords
            .iter()
            .copied()
            .filter(|(x, z)| (x - cx).abs() > SEND_RADIUS || (z - cz).abs() > SEND_RADIUS)
            .collect();
        assert!(
            outside.is_empty(),
            "chunks outside centre ±{SEND_RADIUS}: {outside:?}"
        );
    }

    eprintln!(
        "self-session written: {} packets, center {center:?}, chunks {coords:?}",
        trace.len()
    );
    count_info_lines(&logged.lock().expect("log buffer uncontended"));
}
