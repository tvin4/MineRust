//! The first parity slice: login, configuration and join (play).
//!
//! The same scripted client walks vanilla and our server from handshake all
//! the way into play and records raw traces to
//! `target/parity/join-{vanilla,ours}.json`. The diff of these traces is the
//! owner-facing discrepancy table; nothing is "fixed" before that table is
//! approved.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use parity::{load_parity_env, Client, ParityEnv, Step, Trace, Vanilla};
use protocol::{
    ClientInformation, FinishAck, FinishConfiguration, Handshake, LoginAcknowledged, LoginStart,
    LoginSuccess, NextState, ProtocolState,
};

/// How long to wait for a single scripted packet.
const WAIT: Duration = Duration::from_secs(15);
/// Vanilla answers its known-packs probe within milliseconds; our server
/// sends no probe at all, so this step must not burn the full WAIT.
const PROBE_WAIT: Duration = Duration::from_secs(3);
/// Player name of the scripted client; offline UUIDs derive from it.
const PLAYER: &str = "ParityBot";

/// Reply to vanilla's `select_known_packs` probe: the same single pack the
/// server announced (`minecraft:core` version `26.3`), echoed back in the
/// observed layout (VarInt count, then three strings).
///
/// The layout was read off vanilla's own probe bytes in the parity trace;
/// a successful exchange (finish arrives afterwards) is what proves it.
fn known_packs_reply() -> Vec<u8> {
    let mut out = Vec::new();
    protocol::encode_varint(1, &mut out);
    for part in ["minecraft", "core", "26.3"] {
        protocol::encode_varint(part.len() as i32, &mut out);
        out.extend_from_slice(part.as_bytes());
    }
    out
}

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

/// The scripted login → configuration → play sequence, identical for both
/// servers.
fn join_steps(env: &ParityEnv, ids: &PacketIds, server_port: u16) -> Vec<Step> {
    let handshake = Handshake {
        protocol_version: env.config.protocol_version,
        server_address: "localhost".to_owned(),
        server_port,
        next_state: NextState::Login,
    };
    let login = LoginStart {
        name: PLAYER.to_owned(),
        uuid: protocol::offline_uuid(PLAYER),
    };
    let info = ClientInformation {
        locale: "en_us".to_owned(),
        view_distance: 8,
        chat_mode: 0,
        chat_colors: true,
        skin_parts: 0x7F,
        main_hand: 1,
        text_filtering: false,
        allow_server_listings: true,
        particle_status: 0,
    };
    vec![
        Step::send(
            ids.id(Handshake::KEY),
            handshake.encode_payload().expect("handshake encodable"),
            Some(ProtocolState::Login),
        ),
        Step::send(
            ids.id(LoginStart::KEY),
            login.encode_payload().expect("login encodable"),
            None,
        ),
        Step::wait(ids.id(LoginSuccess::KEY), WAIT),
        Step::send(
            ids.id(LoginAcknowledged::KEY),
            LoginAcknowledged.encode_payload(),
            Some(ProtocolState::Configuration),
        ),
        Step::send(
            ids.id(ClientInformation::KEY),
            info.encode_payload().expect("client info encodable"),
            None,
        ),
        // Vanilla announces its data packs and expects an answer; our
        // server does not perform this exchange, so the step ends quietly.
        Step::respond(
            ids.id(protocol::SELECT_KNOWN_PACKS),
            PROBE_WAIT,
            ids.id(protocol::SELECT_KNOWN_PACKS_SERVERBOUND),
            known_packs_reply(),
        ),
        Step::wait(ids.id(FinishConfiguration::KEY), WAIT),
        Step::send(
            ids.id(FinishAck::KEY),
            FinishAck.encode_payload(),
            Some(ProtocolState::Play),
        ),
        // Everything the server sends on joining: the join sequence and any
        // chunks it decides to send right away. Two seconds of silence ends
        // the recording early.
        Step::quiet(Duration::from_secs(2), Duration::from_secs(45)),
    ]
}

/// Runs the join script against one server and saves the trace.
async fn record_join(
    env: &ParityEnv,
    ids: Arc<PacketIds>,
    target: &str,
    addr: SocketAddr,
    file: &str,
) -> Trace {
    let steps = join_steps(env, &ids, addr.port());
    let mut client = Client::connect(target, addr, Arc::clone(&ids))
        .await
        .expect("connect to the server");
    if let Err(error) = client.run(&steps).await {
        // Keep the partial recording: it shows exactly where the server
        // stopped sending before the script timed out.
        let partial = client.into_trace();
        partial.save(&out_dir().join(file)).ok();
        panic!(
            "{target}: script failed after {} packets: {error}",
            partial.len()
        );
    }
    let trace = client.into_trace();
    trace
        .save(&out_dir().join(file))
        .expect("trace must be written");
    trace
}

/// Kills a stray build `server.exe` so `cargo test` can always relink and
/// the machine is never left running a forgotten server.
///
/// The test's own server runs in-process, but the workspace binary may be
/// left behind by an earlier run (a live check, a crashed test harness) and
/// its open file handle blocks the next `cargo test` build.
fn kill_stray_server_exe() {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/IM", "server.exe"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    {
        // TODO: reach the same guarantee on other platforms.
    }
}

/// Kills stray `server.exe` processes and aborts our in-process server on
/// every exit path of the test, including a failed assertion or panic.
struct ServerGuard {
    /// Handle of the in-process `net::serve` task, if it was started.
    abort: Option<tokio::task::AbortHandle>,
}

impl Drop for ServerGuard {
    fn drop(&mut self) {
        if let Some(abort) = self.abort.take() {
            abort.abort();
        }
        kill_stray_server_exe();
    }
}

/// Starts our own server in-process on an ephemeral port.
///
/// The game data is loaded through the same path the real server uses
/// (client jar + registry report), so the trace records the very packets a
/// running server sends — an in-memory default would record zero registry
/// packets and fake a 32-vs-0 discrepancy in the report.
async fn start_ours(
    env: &ParityEnv,
    ids: Arc<PacketIds>,
) -> (SocketAddr, tokio::task::AbortHandle) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral");
    let addr = listener.local_addr().expect("local addr");
    let status = Arc::new(server::status_info(&env.config));
    let game_data = data::load_from_jar(
        &env.config.client_jar,
        env.config.registries_report.as_deref().map(Path::new),
    )
    .expect("game data from the jar");
    let join = tokio::spawn(net::serve(listener, status, Arc::new(game_data), ids, None));
    (addr, join.abort_handle())
}

/// Asserts the script reached play (Finish Configuration was exchanged).
fn assert_joined(trace: &Trace, ids: &PacketIds) {
    assert!(
        !trace.is_empty(),
        "no packets recorded for {}",
        trace.target
    );
    let finish = ids.id(FinishConfiguration::KEY);
    assert!(
        trace
            .packets
            .iter()
            .any(|entry| entry.id == finish
                && entry.direction == parity::TraceDirection::Clientbound),
        "{}: never reached configuration finish",
        trace.target
    );
    let login = ids.id(protocol::LoginPlay::KEY);
    assert!(
        trace.packets.iter().any(
            |entry| entry.id == login && entry.direction == parity::TraceDirection::Clientbound
        ),
        "{}: no Login (play) received",
        trace.target
    );
}

#[tokio::test]
#[ignore = "requires the owner's vanilla installation (C:\\MineRust-data)"]
async fn join_slice_records_traces_for_both_servers() {
    let env = match parity_env_or_skip() {
        Some(env) => env,
        None => return,
    };
    let ids = Arc::new(env.packet_ids().expect("packet ids"));
    // Active from the very start: whatever fails below, the run cleans up
    // stray `server.exe` processes and its own in-process server.
    let mut guard = ServerGuard { abort: None };

    // Vanilla: start, join, stop.
    let vanilla_port = env.vanilla_port().expect("server-port in properties");
    let vanilla_socket: SocketAddr = format!("127.0.0.1:{vanilla_port}")
        .parse()
        .expect("vanilla address");
    let vanilla = Vanilla::start(&env).await.expect("vanilla starts");
    let vanilla_trace = record_join(
        &env,
        Arc::clone(&ids),
        "vanilla",
        vanilla_socket,
        "join-vanilla.json",
    )
    .await;
    vanilla.stop().await.expect("vanilla stops cleanly");

    // Ours: in-process server on an ephemeral port, released only by the
    // dropping guard so a panic above still tears it down.
    let (ours_addr, abort) = start_ours(&env, Arc::clone(&ids)).await;
    guard.abort = Some(abort);
    let ours_trace = record_join(&env, Arc::clone(&ids), "ours", ours_addr, "join-ours.json").await;
    drop(guard);

    assert_joined(&vanilla_trace, &ids);
    assert_joined(&ours_trace, &ids);
    eprintln!(
        "join traces written to {}: vanilla {} packets, ours {} packets",
        out_dir().display(),
        vanilla_trace.len(),
        ours_trace.len()
    );
}
