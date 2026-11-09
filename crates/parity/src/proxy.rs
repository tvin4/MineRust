//! ЧАСТЬ IV (M4.0 (а), раунд 4): a recording TCP proxy in front of vanilla.
//!
//! The proxy listens on a local address and forwards every byte to the
//! vanilla server, but it frames both directions and writes one JSONL line
//! per packet: the monotonic read and write times, the direction, the
//! protocol state, the packet id, its report name and the base64 payload.
//! The state is switched by three observed packets — `minecraft:intention`
//! (handshake), `minecraft:login_acknowledged` (login → configuration) and
//! `minecraft:finish_configuration` (configuration → play).
//!
//! It is only meaningful with compression off (`network-compression-threshold`
//! negative, so no `Close`/`SetCompression`) and encryption off
//! (`online-mode=false`): with either on the recorded payloads would no
//! longer be plain packets. The caller checks that before starting; the two
//! switch packets (`Set Compression`, the clientbound login `hello`) also
//! abort the connection should they ever appear.

use std::collections::HashMap;
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use data::PacketIds;
use protocol::{
    decode_frame, decode_varint, Direction, FinishConfiguration, Handshake, LoginAcknowledged,
    LoginSuccess, NextState, ProtocolState, SetCompression,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use crate::walk::base64;

/// Everything one proxy run needs.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Address the proxy listens on (the client connects here).
    pub listen: SocketAddr,
    /// The vanilla server the proxy forwards to.
    pub target: SocketAddr,
    /// Directory the per-connection JSONL files are written to.
    pub out_dir: PathBuf,
}

/// Proxy errors that stop the whole run.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    /// The listen address could not be bound.
    #[error("cannot listen on {addr}: {source}")]
    Listen {
        /// Requested listen address.
        addr: SocketAddr,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The output directory could not be created.
    #[error("cannot prepare the output directory {path}: {source}")]
    OutDir {
        /// Requested directory.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

/// The protocol ids the state machine keys on, resolved once.
#[derive(Debug, Clone, Copy)]
struct Ids {
    /// `minecraft:intention` (serverbound, handshake).
    intention: u32,
    /// `minecraft:login_acknowledged` (serverbound, login).
    login_ack: u32,
    /// `minecraft:login_finished` (clientbound, login).
    login_finished: u32,
    /// `minecraft:finish_configuration` (clientbound, configuration).
    finish_configuration: u32,
    /// `minecraft:login_compression` (clientbound, login) — abort if seen.
    set_compression: u32,
}

impl Ids {
    /// Resolves the ids from the loaded packets report (or the compiled
    /// defaults without one).
    fn from(ids: &PacketIds) -> Self {
        Self {
            intention: ids.id(Handshake::KEY),
            login_ack: ids.id(LoginAcknowledged::KEY),
            login_finished: ids.id(LoginSuccess::KEY),
            finish_configuration: ids.id(FinishConfiguration::KEY),
            set_compression: ids.id(SetCompression::KEY),
        }
    }
}

/// Runs the proxy until the process is stopped.
///
/// Never returns on success; each accepted connection is handled on its own
/// task, and a failing connection is logged and dropped without stopping the
/// listener.
pub async fn run_proxy(config: ProxyConfig, ids: Arc<PacketIds>) -> Result<(), ProxyError> {
    std::fs::create_dir_all(&config.out_dir).map_err(|source| ProxyError::OutDir {
        path: config.out_dir.clone(),
        source,
    })?;
    let listener = TcpListener::bind(config.listen)
        .await
        .map_err(|source| ProxyError::Listen {
            addr: config.listen,
            source,
        })?;
    tracing::info!("listening on {} -> {}", config.listen, config.target);
    let names = Arc::new(ids.names_by_id());
    let ids = Ids::from(&ids);
    let mut connection = 0u64;
    loop {
        let (client, peer) = match listener.accept().await {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!("accept failed: {error}");
                continue;
            }
        };
        connection += 1;
        tracing::info!("connection {connection} from {peer}");
        let config = config.clone();
        let names = Arc::clone(&names);
        tokio::spawn(async move {
            if let Err(error) = serve(client, &config, ids, names, connection).await {
                tracing::warn!("connection {connection} ended: {error}");
            }
        });
    }
}

/// Serves one client connection: connect upstream, forward both directions.
async fn serve(
    client: TcpStream,
    config: &ProxyConfig,
    ids: Ids,
    names: Arc<HashMap<(u8, u8, u32), String>>,
    connection: u64,
) -> std::io::Result<()> {
    let upstream = TcpStream::connect(config.target).await?;
    client.set_nodelay(true)?;
    upstream.set_nodelay(true)?;
    let path = config
        .out_dir
        .join(format!("proxy-conn-{connection}.jsonl"));
    let file = std::fs::File::create(&path)?;
    tracing::info!("recording {}", path.display());
    let started = Instant::now();
    let shared = Arc::new(Mutex::new(Recording {
        file: std::io::BufWriter::new(file),
        ids,
        names,
        state: ProtocolState::Handshake,
        seq: 0,
    }));
    let (client_read, client_write) = client.into_split();
    let (server_read, server_write) = upstream.into_split();
    let mut to_server = tokio::spawn(forward(
        client_read,
        server_write,
        Direction::Serverbound,
        Arc::clone(&shared),
        started,
    ));
    let mut to_client = tokio::spawn(forward(
        server_read,
        client_write,
        Direction::Clientbound,
        Arc::clone(&shared),
        started,
    ));
    tokio::select! {
        _ = &mut to_server => to_client.abort(),
        _ = &mut to_client => to_server.abort(),
    }
    let _ = shared.lock().await.file.flush();
    Ok(())
}

/// Forwards one direction frame by frame, recording every packet.
async fn forward(
    mut read: OwnedReadHalf,
    mut write: OwnedWriteHalf,
    direction: Direction,
    shared: Arc<Mutex<Recording>>,
    started: Instant,
) -> std::io::Result<()> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let read_count = read.read(&mut chunk).await?;
        if read_count == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read_count]);
        while let Some(frame) = take_frame(&mut buffer)? {
            let read_ms = milliseconds(started.elapsed());
            let (state, name, abort) = {
                let mut recording = shared.lock().await;
                recording.classify(direction, &frame.body)
            };
            write.write_all(&frame.raw).await?;
            write.flush().await?;
            let write_ms = milliseconds(started.elapsed());
            {
                let mut recording = shared.lock().await;
                recording.log(
                    read_ms,
                    write_ms,
                    direction,
                    state,
                    frame.id,
                    &name,
                    &frame.payload,
                )?;
            }
            if let Some(reason) = abort {
                return Err(std::io::Error::other(reason));
            }
        }
    }
    Ok(())
}

/// One complete packet taken off the wire.
struct Frame {
    /// The packet as it arrived, length prefix included (forwarded verbatim).
    raw: Vec<u8>,
    /// The numeric packet id.
    id: u32,
    /// The id and the payload, length prefix removed (for the state machine).
    body: Vec<u8>,
    /// The payload only, id removed (what the JSONL carries in base64).
    payload: Vec<u8>,
}

/// Takes one whole frame out of the buffer, or `None` when it is incomplete.
fn take_frame(buffer: &mut Vec<u8>) -> std::io::Result<Option<Frame>> {
    let Some((length, prefix)) = try_varint(buffer)? else {
        return Ok(None);
    };
    if length < 0 {
        return Err(std::io::Error::other(format!(
            "negative frame length {length}"
        )));
    }
    let total = prefix + length as usize;
    if buffer.len() < total {
        return Ok(None);
    }
    let raw = buffer[..total].to_vec();
    let body = buffer[prefix..total].to_vec();
    let (id, id_len) = decode_varint(&body)
        .map_err(|error| std::io::Error::other(format!("packet id: {error}")))?;
    let payload = body[id_len..].to_vec();
    buffer.drain(..total);
    Ok(Some(Frame {
        raw,
        id: id as u32,
        body,
        payload,
    }))
}

/// Decodes the VarInt at the front of `buffer`; `None` means it is cut off.
fn try_varint(buffer: &[u8]) -> std::io::Result<Option<(i32, usize)>> {
    let mut value: u32 = 0;
    let mut shift = 0;
    for (index, &byte) in buffer.iter().enumerate() {
        if index >= 5 {
            return Err(std::io::Error::other("VarInt is longer than five bytes"));
        }
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(Some((value as i32, index + 1)));
        }
        shift += 7;
    }
    Ok(None)
}

/// The per-connection recording state: file, names, protocol state, sequence.
struct Recording {
    /// Buffered JSONL file; flushed when the connection ends.
    file: std::io::BufWriter<std::fs::File>,
    /// Resolved ids of the packets that drive the state machine.
    ids: Ids,
    /// `(state, direction, id)` → report name (empty without a report).
    names: Arc<HashMap<(u8, u8, u32), String>>,
    /// Protocol state the next packet of either direction belongs to.
    state: ProtocolState,
    /// JSONL row counter.
    seq: u64,
}

impl Recording {
    /// Names a packet by the state it belongs to and advances the state.
    ///
    /// Returns the state the packet was recorded in, its name, and an abort
    /// reason when the packet proves compression or encryption is on.
    fn classify(
        &mut self,
        direction: Direction,
        body: &[u8],
    ) -> (ProtocolState, String, Option<String>) {
        let state = self.state;
        let id = decode_frame(body).map(|(id, _)| id).unwrap_or(u32::MAX);
        let name = self
            .names
            .get(&(state.index() as u8, direction.index() as u8, id))
            .cloned()
            .unwrap_or_default();
        let key = (direction, state, id);
        let mut abort = None;
        if direction == Direction::Clientbound
            && state == ProtocolState::Login
            && id == self.ids.set_compression
        {
            abort = Some("compression is on (Set Compression seen)".to_owned());
        }
        if direction == Direction::Clientbound
            && state == ProtocolState::Login
            && name == "minecraft:hello"
        {
            abort = Some("encryption is on (clientbound login hello seen)".to_owned());
        }
        match key {
            (Direction::Serverbound, ProtocolState::Handshake, id) if id == self.ids.intention => {
                let next = decode_frame(body)
                    .ok()
                    .and_then(|(_, mut reader)| Handshake::decode_payload(&mut reader).ok())
                    .map(|handshake| handshake.next_state);
                self.state = match next {
                    Some(NextState::Status) => ProtocolState::Status,
                    _ => ProtocolState::Login,
                };
            }
            (Direction::Serverbound, ProtocolState::Login, id) if id == self.ids.login_ack => {
                self.state = ProtocolState::Configuration;
            }
            (Direction::Clientbound, ProtocolState::Login, id) if id == self.ids.login_finished => {
                self.state = ProtocolState::Configuration;
            }
            (Direction::Clientbound, ProtocolState::Configuration, id)
                if id == self.ids.finish_configuration =>
            {
                self.state = ProtocolState::Play;
            }
            _ => {}
        }
        (state, name, abort)
    }

    /// Appends one JSONL row for a packet that was read and then written.
    #[allow(clippy::too_many_arguments)]
    fn log(
        &mut self,
        read_ms: f64,
        write_ms: f64,
        direction: Direction,
        state: ProtocolState,
        id: u32,
        name: &str,
        payload: &[u8],
    ) -> std::io::Result<()> {
        self.seq += 1;
        let line = serde_json::json!({
            "seq": self.seq,
            "read_ms": read_ms,
            "write_ms": write_ms,
            "dir": direction.as_str(),
            "state": state.as_str(),
            "id": id,
            "name": name,
            "b64": base64(payload),
        });
        serde_json::to_writer(&mut self.file, &line)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        self.file.write_all(b"\n")?;
        Ok(())
    }
}

/// Milliseconds of a duration as a float (the trace convention).
fn milliseconds(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_complete_frame_is_split_into_id_and_payload() {
        // Length 3, id 0x2a, payload [1, 2].
        let mut buffer = vec![0x03, 0x2a, 0x01, 0x02];
        let frame = take_frame(&mut buffer)
            .expect("no io error")
            .expect("a frame");
        assert_eq!(frame.id, 0x2a);
        assert_eq!(frame.payload, vec![0x01, 0x02]);
        assert_eq!(frame.raw, vec![0x03, 0x2a, 0x01, 0x02]);
        assert!(buffer.is_empty());
    }

    #[test]
    fn an_incomplete_frame_is_left_in_the_buffer() {
        let mut buffer = vec![0x03, 0x2a, 0x01];
        assert!(take_frame(&mut buffer).expect("no io error").is_none());
        buffer.push(0x02);
        assert!(take_frame(&mut buffer).expect("no io error").is_some());
    }

    #[test]
    fn two_frames_in_one_buffer_come_out_in_order() {
        let mut buffer = vec![0x01, 0x00, 0x01, 0x07];
        let first = take_frame(&mut buffer)
            .expect("no io error")
            .expect("first");
        let second = take_frame(&mut buffer)
            .expect("no io error")
            .expect("second");
        assert_eq!(first.id, 0x00);
        assert_eq!(second.id, 0x07);
        assert!(buffer.is_empty());
    }

    #[test]
    fn a_varint_longer_than_five_bytes_is_an_error() {
        let buffer = [0x80, 0x80, 0x80, 0x80, 0x80, 0x01];
        assert!(try_varint(&buffer).is_err());
    }
}
