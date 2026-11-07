//! Slice 1 of the parity plan: server list ping against vanilla and ours.
//!
//! The same scripted client runs twice — against the official `server.jar`
//! and against our own server in-process — and records raw packet traces to
//! `target/parity/status-{vanilla,ours}.json` for the later diff.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use parity::{load_parity_env, Client, ParityEnv, Step, Trace, Vanilla};
use protocol::{
    Handshake, NextState, PingRequest, PongResponse, ProtocolState, StatusRequest, StatusResponse,
};

/// How long to wait for a single status packet.
const WAIT: Duration = Duration::from_secs(10);

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

/// The scripted status exchange, identical for both servers.
fn status_steps(env: &ParityEnv, ids: &PacketIds, server_port: u16) -> Vec<Step> {
    let handshake = Handshake {
        protocol_version: env.config.protocol_version,
        server_address: "localhost".to_owned(),
        server_port,
        next_state: NextState::Status,
    };
    let ping = PingRequest {
        payload: 0x0102_0304_0506_0708,
    };
    vec![
        Step::send(
            ids.id(Handshake::KEY),
            handshake.encode_payload().expect("handshake encodable"),
            Some(ProtocolState::Status),
        ),
        Step::send(
            ids.id(StatusRequest::KEY),
            StatusRequest.encode_payload(),
            None,
        ),
        Step::wait(ids.id(StatusResponse::KEY), WAIT),
        Step::send(ids.id(PingRequest::KEY), ping.encode_payload(), None),
        Step::wait(ids.id(PongResponse::KEY), WAIT),
    ]
}

/// Runs the script against one server and saves the trace.
async fn record_status(
    env: &ParityEnv,
    ids: Arc<PacketIds>,
    target: &str,
    addr: SocketAddr,
    file: &str,
) -> Trace {
    let steps = status_steps(env, &ids, addr.port());
    let mut client = Client::connect(target, addr, Arc::clone(&ids))
        .await
        .expect("connect to the server");
    client.run(&steps).await.expect("run the status script");
    let trace = client.into_trace();
    trace
        .save(&out_dir().join(file))
        .expect("trace must be written");
    trace
}

/// Starts our own server in-process on an ephemeral port.
async fn start_ours(
    env: &ParityEnv,
    ids: Arc<PacketIds>,
) -> (SocketAddr, tokio::task::AbortHandle) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr = listener.local_addr().expect("local addr");
    let status = Arc::new(server::status_info(&env.config));
    let join = tokio::spawn(net::serve(
        listener,
        status,
        Arc::new(data::GameData::default()),
        ids,
    ));
    (addr, join.abort_handle())
}

/// Asserts a trace looks like a completed status exchange.
fn assert_status_trace(trace: &Trace, ids: &PacketIds) {
    assert!(
        !trace.is_empty(),
        "no packets recorded for {}",
        trace.target
    );
    let response = ids.id(StatusResponse::KEY);
    let pong = ids.id(PongResponse::KEY);
    assert!(
        trace.packets.iter().any(
            |entry| entry.direction == parity::TraceDirection::Clientbound && entry.id == response
        ),
        "{}: no status response",
        trace.target
    );
    assert!(
        trace.packets.iter().any(|entry| entry.id == pong),
        "{}: no pong",
        trace.target
    );
}

#[tokio::test]
#[ignore = "requires the owner's vanilla installation (C:\\MineRust-data)"]
async fn status_slice_records_traces_for_both_servers() {
    let env = match parity_env_or_skip() {
        Some(env) => env,
        None => return,
    };
    let ids = Arc::new(env.packet_ids().expect("packet ids"));

    // Vanilla: start, ping, stop.
    let vanilla_port = env.vanilla_port().expect("server-port in properties");
    let vanilla_socket: SocketAddr = format!("127.0.0.1:{vanilla_port}")
        .parse()
        .expect("vanilla address");
    let vanilla = Vanilla::start(&env).await.expect("vanilla starts");
    let vanilla_trace = record_status(
        &env,
        Arc::clone(&ids),
        "vanilla",
        vanilla_socket,
        "status-vanilla.json",
    )
    .await;
    vanilla.stop().await.expect("vanilla stops cleanly");

    // Ours: in-process server on an ephemeral port.
    let (ours_addr, abort) = start_ours(&env, Arc::clone(&ids)).await;
    let ours_trace = record_status(
        &env,
        Arc::clone(&ids),
        "ours",
        ours_addr,
        "status-ours.json",
    )
    .await;
    abort.abort();

    assert_status_trace(&vanilla_trace, &ids);
    assert_status_trace(&ours_trace, &ids);
    eprintln!(
        "traces written to {}: vanilla {} packets, ours {} packets",
        out_dir().display(),
        vanilla_trace.len(),
        ours_trace.len()
    );
}
