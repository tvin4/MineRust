//! Step 3.3d: the composition of the serverbound `chunk_batch_received`.
//!
//! The documented packet (SOURCE: wiki, unverified) carries a single
//! float, "Chunks Per Tick". The check the owner prescribed: a bot joins
//! vanilla, waits for the `chunk_batch_finished` of the join batch,
//! answers it with that single float — and a server that neither closes
//! the connection nor stops streaming after the unanswered reply has
//! confirmed the composition. The keep-alive that arrives afterwards (a
//! ~15 s cadence) is the proof; the confirmed field goes to
//! `docs/SOURCES.md`.
//!
//! The recorded trace lands in `target/parity/chunk-batch-vanilla.json`.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use parity::{join_steps, load_parity_env, Client, ParityEnv, Step, Trace, Vanilla};
use protocol::{ChunkBatchFinished, ChunkBatchReceived, KeepAlive};

/// How long the scratch bot waits for the finished marker of the batch it
/// is answering (vanilla sends its join chunks within milliseconds).
const BATCH_WAIT: Duration = Duration::from_secs(15);
/// How long it watches the stream after sending the pace. Vanilla's
/// keep-alives are about fifteen seconds apart, so a second one may never
/// be needed; sixty seconds fits the cadence plus the join comfortably.
const STREAM_WAIT: Duration = Duration::from_secs(60);

/// Reads the parity environment or explains why the test is skipped.
fn parity_env_or_skip() -> Option<ParityEnv> {
    match load_parity_env() {
        Ok(env) => Some(env),
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            None
        }
    }
}

/// Where traces land: `<workspace>/target/parity/`.
fn out_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("parity")
}

/// The script: join vanilla, wait for the join batch's finished marker,
/// answer it with the single float of step 3.3d, then watch the stream
/// for the next keep-alive.
fn pace_steps(env: &ParityEnv, ids: &PacketIds, server_port: u16) -> Vec<Step> {
    let mut steps = join_steps(env, ids, server_port);
    steps.push(Step::wait(ids.id(ChunkBatchFinished::KEY), BATCH_WAIT));
    steps.push(Step::send(
        ids.id(ChunkBatchReceived::KEY),
        ChunkBatchReceived {
            chunks_per_tick: 8.0,
        }
        .encode_payload(),
        None,
    ));
    // Only a connection still open *and* still talking passes this step:
    // a rejection would either close (the read errors out) or stop the
    // cadence (the wait times out).
    steps.push(Step::wait(ids.id(KeepAlive::KEY), STREAM_WAIT));
    steps
}

/// Runs the pace script against vanilla and saves the trace.
async fn record_pace(env: &ParityEnv, ids: Arc<PacketIds>, addr: SocketAddr) -> Trace {
    let steps = pace_steps(env, &ids, addr.port());
    let mut client = Client::connect("vanilla", addr, Arc::clone(&ids))
        .await
        .expect("connect to the server");
    let file = out_dir().join("chunk-batch-vanilla.json");
    if let Err(error) = client.run(&steps).await {
        // Keep the partial recording: it shows exactly where the server
        // stopped before the script timed out.
        let partial = client.into_trace();
        partial.save(&file).ok();
        panic!(
            "vanilla: the pace script stopped after {} packets: {error}",
            partial.len()
        );
    }
    let trace = client.into_trace();
    trace.save(&file).expect("trace must be written");
    trace
}

/// The owner's check of step 3.3d — see the module docs.
#[tokio::test]
#[ignore = "requires the owner's vanilla installation (C:\\MineRust-data)"]
async fn vanilla_accepts_the_single_float_pace_and_keeps_streaming() {
    let env = match parity_env_or_skip() {
        Some(env) => env,
        None => return,
    };
    let ids = Arc::new(env.packet_ids().expect("packet ids"));
    let vanilla_port = env.vanilla_port().expect("server-port in properties");
    let vanilla_socket: SocketAddr = format!("127.0.0.1:{vanilla_port}")
        .parse()
        .expect("vanilla address");
    let vanilla = Vanilla::start(&env).await.expect("vanilla starts");
    let trace = record_pace(&env, Arc::clone(&ids), vanilla_socket).await;
    vanilla.stop().await.expect("vanilla stops cleanly");

    // The bot really sent the composition under test (the script's own
    // success is the server having accepted it).
    let sent = trace.packets.iter().any(|entry| {
        entry.id == ids.id(ChunkBatchReceived::KEY)
            && entry.direction == parity::TraceDirection::Serverbound
    });
    assert!(sent, "the pace answer must have been recorded as sent");
    eprintln!(
        "CONFIRMED: vanilla kept streaming after a single-float \
         chunk_batch_received; trace at {}",
        out_dir().join("chunk-batch-vanilla.json").display()
    );
}
