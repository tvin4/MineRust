//! ЧАСТЬ 2 (M4.4 A): our own server's join window against the traced shape.
//!
//! The bot joins the in-process server on the configured world (the v2
//! template, byte-identical to `world-template`) and stands still exactly
//! like the ЧАСТЬ 0 runs did against vanilla. Two facts are asserted, no
//! payload is compared (light is a known debt of its own):
//!
//! * the R=10 join sends the traced 473-chunk window and nothing else —
//!   provided the whole window is inside the v2 ready batch (it is: the
//!   template is 27×27 and the window half-width is 11);
//! * the R=8 join sends the traced 329-chunk window (the `K(R)` formula of
//!   M4.4 B applies at R=8 too) — provided the whole window is inside the
//!   v2 ready batch (it is: the template is 27×27 and the window
//!   half-width is 9);
//! * the send order is our own `(dx²+dz², x, z)`, element for element the
//!   order [`net::window`] returns, both radii.
//!
//! The traces land in `C:\MineRust-data\traces\join-window-ours-r{10,8}.jsonl`
//! so `python tools/trace-analyze/analyze.py shape-join <trace> --list` can
//! read them back and diff our R=10 set against the vanilla control.
//!
//! ```text
//! cargo test -p parity --test join_window_ours -- --ignored --nocapture
//! ```

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use parity::{load_parity_env, run_walk_view_at, ParityEnv, Stop, WalkMode};

/// Where the traces land, next to the ЧАСТЬ 0 vanilla traces.
const TRACE_DIR: &str = r"C:\MineRust-data\traces";

/// The owner's idle rule and hard budget, the same as the ЧАСТЬ 0 runs.
const IDLE: Duration = Duration::from_secs(5);
const TIMEOUT: Duration = Duration::from_secs(60);

/// The spawn chunk of the v2 template: the centre every window is built
/// around (`WorldInfo.spawn >> 4`), the same (6, −2) ЧАСТЬ 0 measured.
const CENTRE: (i32, i32) = (6, -2);

/// One case: the radius, how many chunks the traced window holds and the
/// trace file name.
struct Case {
    radius: i8,
    expected: usize,
}

#[tokio::test]
#[ignore = "requires the owner's installation (C:\\MineRust-data)"]
async fn our_join_window_matches_the_traced_shape() {
    let env = match load_parity_env() {
        Ok(env) => env,
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            return;
        }
    };
    if let Err(error) = std::fs::create_dir_all(TRACE_DIR) {
        eprintln!("SKIP: cannot create {TRACE_DIR}: {error}");
        return;
    }
    let ids = Arc::new(env.packet_ids().expect("packet ids"));
    let (addr, abort) = start_with_world(&env, Arc::clone(&ids)).await;
    // Active from the very start: a failed assertion below still aborts the
    // in-process server.
    let _guard = ServerGuard { abort: Some(abort) };

    // R=10 and R=8 are both traced radii: the `K(R)` formula (M4.4 B)
    // gives 473 and 329.
    let cases = [
        Case {
            radius: 10,
            expected: 473,
        },
        Case {
            radius: 8,
            expected: 329,
        },
    ];
    for case in cases {
        check_one(&env, Arc::clone(&ids), addr, &case).await;
    }
}

/// One radius: run the join window against our server, then assert its set
/// and order against [`net::window`].
async fn check_one(env: &ParityEnv, ids: Arc<data::PacketIds>, addr: SocketAddr, case: &Case) {
    let trace = Path::new(TRACE_DIR).join(format!("join-window-ours-r{}.jsonl", case.radius));
    let _ = std::fs::remove_file(&trace);
    let mode = WalkMode::JoinWindow {
        idle: IDLE,
        timeout: TIMEOUT,
    };
    let stats = match run_walk_view_at(env, ids, addr.port(), case.radius, mode, &trace).await {
        Ok(stats) => stats,
        Err(error) => panic!("r={}: walk failed: {error}", case.radius),
    };
    assert!(
        matches!(stats.stop, Stop::WindowIdle { .. }),
        "r={}: the run must end on the idle gap, not {:?}",
        case.radius,
        stats.stop
    );

    let sent = read_chunk_coords(&trace);
    let wanted = net::window(CENTRE.0, CENTRE.1, i32::from(case.radius));
    let wanted_set: BTreeSet<(i32, i32)> = wanted.iter().copied().collect();
    let sent_set: BTreeSet<(i32, i32)> = sent.iter().copied().collect();
    let outside: Vec<(i32, i32)> = sent_set.difference(&wanted_set).copied().collect();
    let missing: Vec<(i32, i32)> = wanted_set.difference(&sent_set).copied().collect();

    eprintln!(
        "r={}: sent {} (stats.chunks {}), in_window {}, not_in_v2 {}, outside {}, order {}",
        case.radius,
        sent.len(),
        stats.chunks,
        wanted.len(),
        missing.len(),
        outside.len(),
        if sent == wanted { "exact" } else { "differs" }
    );

    assert_eq!(
        sent.len(),
        case.expected,
        "r={}: the traced count of the join window",
        case.radius
    );
    assert_eq!(
        stats.chunks as usize, case.expected,
        "r={}: every window chunk reached the wire",
        case.radius
    );
    assert!(
        outside.is_empty(),
        "r={}: chunks outside the window: {outside:?}",
        case.radius
    );
    assert!(
        missing.is_empty(),
        "r={}: window chunks absent from v2: {missing:?}",
        case.radius
    );
    // With 0 missing, the resolved window goes out in exactly the order
    // `net::window` returns it: `(dx²+dz², x, z)`, our tie-break.
    assert_eq!(
        sent, wanted,
        "r={}: the send order is our `(dx^2+dz^2, x, z)` order",
        case.radius
    );
}

/// Chunk coordinates of every `level_chunk_with_light` in a walk JSONL
/// trace, in arrival (seq) order: the first two big-endian `i32` of the
/// payload, the layout the analyzer also reads.
fn read_chunk_coords(path: &Path) -> Vec<(i32, i32)> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read trace {}: {error}", path.display()));
    let mut rows: Vec<(u64, (i32, i32))> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let row: serde_json::Value = serde_json::from_str(line).expect("trace line is JSON");
        if row["name"] != "minecraft:level_chunk_with_light" {
            continue;
        }
        let seq = row["seq"].as_u64().expect("seq");
        let bytes = base64_decode(row["b64"].as_str().expect("b64"))
            .unwrap_or_else(|| panic!("chunk payload {} is base64", seq));
        assert!(bytes.len() >= 8, "chunk payload {seq} carries x and z");
        let x = i32::from_be_bytes(bytes[0..4].try_into().expect("big endian x"));
        let z = i32::from_be_bytes(bytes[4..8].try_into().expect("big endian z"));
        rows.push((seq, (x, z)));
    }
    rows.sort_by_key(|(seq, _)| *seq);
    rows.into_iter().map(|(_, coord)| coord).collect()
}

/// Standard base64 with padding, the JSONL payload encoding.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    fn value(byte: u8) -> Option<u32> {
        match byte {
            b'A'..=b'Z' => Some(u32::from(byte - b'A')),
            b'a'..=b'z' => Some(u32::from(byte - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(byte - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut acc = 0_u32;
    let mut bits = 0_u32;
    for &byte in text.as_bytes() {
        if byte == b'=' {
            break;
        }
        let value = value(byte)?;
        acc = (acc << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Starts our own server in-process on the loaded world at an ephemeral
/// port (the parity self-session pattern: `server::build_world` + the game
/// data from the jar, so the wire is the production one).
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
    let (loader, world) = server::build_world(&env.config, Arc::clone(&game_data))
        .unwrap_or_else(|error| panic!("the saved world cannot be built: {error}"));
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
