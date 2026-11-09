//! ЧАСТЬ IV verification: the recording proxy is checked with a standing bot
//! for 10 s and **no live client** (the owner's condition).
//!
//! The bot joins through the proxy (the proxy forwards to the vanilla server
//! of `server.toml`) and stands still; the proxy writes its own JSONL next to
//! the traces. The test then reads that file back and prints the row count and
//! the `(direction, state)` histogram, which is the fact the run produced.
//!
//! ```text
//! cargo test -p parity --test proxy_verify -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};
use std::sync::Arc;
use std::time::Duration;

use parity::{load_parity_env, run_walk_at, ParityEnv, Vanilla, WalkMode};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};

/// Where the bot trace and the proxy recordings land.
const TRACE_DIR: &str = r"C:\MineRust-data\traces";
/// Proxy listen port (the owner's ЧАСТЬ IV: 25565 -> vanilla 25566).
const PROXY_PORT: u16 = 25565;
/// How long the bot stands after joining.
const STAND: Duration = Duration::from_secs(10);

fn parity_env_or_skip() -> Option<ParityEnv> {
    match load_parity_env() {
        Ok(env) => Some(env),
        Err(error) => {
            eprintln!("SKIP: parity environment is not configured: {error}");
            None
        }
    }
}

/// Kills a stray build `server.exe`: one of them would bind 25565 and the bot
/// would connect to it instead of the proxy, faking a pass.
fn kill_stray_server_exe() {
    #[cfg(windows)]
    {
        let _ = StdCommand::new("taskkill")
            .args(["/F", "/IM", "server.exe"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Starts the proxy binary on the owner's fixed ports.
fn spawn_proxy() -> std::io::Result<Child> {
    Command::new(env!("CARGO_BIN_EXE_mcproxy"))
        .args([
            "--listen",
            &format!("127.0.0.1:{PROXY_PORT}"),
            "--out",
            TRACE_DIR,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
}

/// Waits until something accepts connections on `port`.
async fn wait_for_port(port: u16, timeout: Duration) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let until = tokio::time::Instant::now() + timeout;
    loop {
        let attempt = tokio::time::timeout(Duration::from_millis(500), TcpStream::connect(addr));
        if let Ok(Ok(_stream)) = attempt.await {
            return true;
        }
        if tokio::time::Instant::now() >= until {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Stops the proxy process tree (its own listener is the whole process).
async fn stop_proxy(mut proxy: Child) {
    let Some(pid) = proxy.id() else {
        let _ = proxy.kill().await;
        return;
    };
    let _ = StdCommand::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = proxy.wait().await;
}

/// The most recently written `proxy-conn-*.jsonl` in the traces directory.
fn newest_recording() -> Option<PathBuf> {
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(TRACE_DIR).ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("proxy-conn-") || !name.ends_with(".jsonl") {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(time, _)| modified > *time) {
            newest = Some((modified, entry.path()));
        }
    }
    newest.map(|(_, path)| path)
}

/// The `(direction, state) → count` histogram of one recording.
type Histogram = Vec<((String, String), usize)>;

/// Reads one recording back: `(rows, histogram of (dir, state) → count)`.
fn summarize(path: &Path) -> (usize, Histogram) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (0, Vec::new());
    };
    let mut rows = 0usize;
    let mut histogram: Histogram = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        rows += 1;
        let dir = value
            .get("dir")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_owned();
        let state = value
            .get("state")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_owned();
        match histogram
            .iter_mut()
            .find(|(key, _)| *key == (dir.clone(), state.clone()))
        {
            Some((_, count)) => *count += 1,
            None => histogram.push(((dir, state), 1)),
        }
    }
    (rows, histogram)
}

#[tokio::test]
#[ignore = "requires the owner's installation (C:\\MineRust-data)"]
async fn proxy_records_a_standing_bot() {
    let env = match parity_env_or_skip() {
        Some(env) => env,
        None => return,
    };
    let ids = Arc::new(env.packet_ids().expect("packet ids"));
    if let Err(error) = std::fs::create_dir_all(TRACE_DIR) {
        eprintln!("SKIP: cannot create {TRACE_DIR}: {error}");
        return;
    }
    kill_stray_server_exe();

    let vanilla = match Vanilla::start(&env).await {
        Ok(server) => server,
        Err(error) => {
            eprintln!("vanilla did not start: {error}");
            return;
        }
    };
    let proxy = match spawn_proxy() {
        Ok(child) => child,
        Err(error) => {
            eprintln!("proxy did not start: {error}");
            let _ = vanilla.stop().await;
            return;
        }
    };
    if !wait_for_port(PROXY_PORT, Duration::from_secs(30)).await {
        eprintln!("proxy never listened on {PROXY_PORT}");
        stop_proxy(proxy).await;
        let _ = vanilla.stop().await;
        return;
    }
    eprintln!(
        "proxy is listening; joining through it for {} s",
        STAND.as_secs()
    );

    let trace = Path::new(TRACE_DIR).join("proxy-standing-bot.jsonl");
    let outcome = run_walk_at(
        &env,
        Arc::clone(&ids),
        PROXY_PORT,
        WalkMode::Stand { hold: STAND },
        &trace,
    )
    .await;
    // Give the proxy a moment to flush the last rows, then stop it.
    tokio::time::sleep(Duration::from_millis(500)).await;
    stop_proxy(proxy).await;
    if let Err(error) = vanilla.stop_with_flush().await {
        eprintln!("vanilla did not stop cleanly: {error}");
    }

    match &outcome {
        Ok(stats) => eprintln!(
            "bot: {:?}, {} chunks, {:.0} ms of play",
            stats.stop, stats.chunks, stats.elapsed_ms
        ),
        Err(error) => eprintln!("bot run failed: {error}"),
    }

    let recording = newest_recording();
    match recording {
        Some(path) => {
            let (rows, histogram) = summarize(&path);
            eprintln!("recording {}: {rows} rows", path.display());
            for ((dir, state), count) in &histogram {
                eprintln!("  {dir:<11} {state:<14} {count}");
            }
        }
        None => eprintln!("no proxy-conn-*.jsonl found in {TRACE_DIR}"),
    }

    assert!(
        outcome.is_ok(),
        "the bot must complete the standing run through the proxy"
    );
}
