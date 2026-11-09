//! ЧАСТЬ IV (M4.0 (а), раунд 4): the recording proxy binary.
//!
//! Listens on `--listen` (default `127.0.0.1:25565`) and forwards to the
//! vanilla server named by `server.toml` (the port from its
//! `server.properties`), writing one JSONL per connection into `--out`
//! (default `C:\MineRust-data\traces`).
//!
//! The proxy refuses to run with compression or encryption on: both would
//! change the packets, so the recorded payloads would be misleading.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use parity::{load_parity_env, run_proxy, ProxyConfig};

/// Where the client connects unless `--listen` overrides it.
const DEFAULT_LISTEN: &str = "127.0.0.1:25565";
/// Where the recordings land unless `--out` overrides it.
const DEFAULT_OUT: &str = r"C:\MineRust-data\traces";

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let args: Vec<String> = std::env::args().collect();
    let listen = argument(&args, "--listen").unwrap_or_else(|| DEFAULT_LISTEN.to_owned());
    let out = argument(&args, "--out").unwrap_or_else(|| DEFAULT_OUT.to_owned());

    let env = match load_parity_env() {
        Ok(env) => env,
        Err(error) => fail(&format!("parity environment: {error}")),
    };
    let port = match env.vanilla_port() {
        Ok(port) => port,
        Err(error) => fail(&format!("vanilla port: {error}")),
    };
    if let Err(error) = check_plain(&env.vanilla_dir) {
        fail(&error);
    }
    let listen: SocketAddr = match listen.parse() {
        Ok(addr) => addr,
        Err(error) => fail(&format!("bad --listen {listen}: {error}")),
    };
    let target: SocketAddr = match format!("127.0.0.1:{port}").parse() {
        Ok(addr) => addr,
        Err(error) => fail(&format!("bad target address: {error}")),
    };
    let ids = match env.packet_ids() {
        Ok(ids) => Arc::new(ids),
        Err(error) => fail(&format!("packet ids: {error}")),
    };
    let config = ProxyConfig {
        listen,
        target,
        out_dir: PathBuf::from(out),
    };
    tracing::info!(
        "recording {} -> {} into {}",
        config.listen,
        config.target,
        config.out_dir.display()
    );
    if let Err(error) = run_proxy(config, ids).await {
        fail(&format!("{error}"));
    }
}

/// Looks up `--flag value` in the arguments.
fn argument(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|item| item == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

/// Refuses compression and encryption (ЧАСТЬ IV): both would rewrite packets.
fn check_plain(vanilla_dir: &std::path::Path) -> Result<(), String> {
    let path = vanilla_dir.join("server.properties");
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let property = |key: &str| {
        text.lines()
            .map(str::trim)
            .find_map(|line| line.strip_prefix(key))
            .map(str::to_owned)
    };
    let threshold = property("network-compression-threshold=").unwrap_or_default();
    let online = property("online-mode=").unwrap_or_default();
    let threshold: i32 = threshold.parse().unwrap_or(0);
    if threshold >= 0 {
        return Err(format!(
            "compression is on (network-compression-threshold={threshold}); \
             the proxy needs a negative threshold"
        ));
    }
    if online != "false" {
        return Err(format!(
            "encryption is on (online-mode={online}); the proxy needs online-mode=false"
        ));
    }
    Ok(())
}

/// Prints the reason and exits with a non-zero code.
fn fail(message: &str) -> ! {
    eprintln!("proxy: {message}");
    std::process::exit(2);
}
