//! The scripted parity test client: dumb steps in, raw trace out.
//!
//! The client has no gameplay logic. It executes a fixed list of [`Step`]s
//! (send this packet, wait for that one, read until the line goes quiet) and
//! records every packet it sends or receives as raw
//! `(state, direction, id, payload)` into a [`Trace`]. The only intelligence
//! is transport-level and strictly required to read the wire: applying the
//! server's Set Compression threshold once vanilla sends it.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use data::PacketIds;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use protocol::{decode_varint, encode_varint, Direction, KeepAlive, ProtocolState, SetCompression};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::Instant;

use crate::trace::{Trace, TraceEntry};

/// Sanity cap for an incoming frame body; real packets stay far below it.
const MAX_BODY: usize = 16 * 1024 * 1024;

/// One action of the client script. The script itself lives in the test.
///
/// Every step is unconditional except [`Step::Respond`], which covers the
/// single optional exchange the parity plan needs: vanilla's known-packs
/// probe does not exist in our own server (Variant A), so the script must be
/// able to walk past it without failing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Send a packet; `to` switches the protocol state afterwards.
    Send {
        /// Numeric packet id (resolved through the packets report).
        id: u32,
        /// Raw payload, packet id excluded.
        payload: Vec<u8>,
        /// State to enter after sending, if any.
        to: Option<ProtocolState>,
    },
    /// Record packets until one with this id arrives (included), or fail.
    Wait {
        /// Serverbound-side id to wait for.
        id: u32,
        /// How long to wait for it.
        timeout: Duration,
    },
    /// Answer a server probe if it arrives within `timeout`; when it does
    /// not (our server sends no probe), the step just ends quietly.
    Respond {
        /// Serverbound-side id being answered.
        id: u32,
        /// How long to wait for the probe.
        timeout: Duration,
        /// Packet id of the reply.
        reply_id: u32,
        /// Raw payload of the reply.
        reply_payload: Vec<u8>,
    },
    /// Record packets until the line has been quiet for `quiet`, but never
    /// longer than `limit` in total; succeeds either way.
    Quiet {
        /// Silence that ends the read.
        quiet: Duration,
        /// Hard cap for the whole step.
        limit: Duration,
    },
}

impl Step {
    /// A send step; see [`Step::Send`].
    pub fn send(id: u32, payload: Vec<u8>, to: Option<ProtocolState>) -> Self {
        Self::Send { id, payload, to }
    }

    /// A wait step; see [`Step::Wait`].
    pub fn wait(id: u32, timeout: Duration) -> Self {
        Self::Wait { id, timeout }
    }

    /// A probe-answer step; see [`Step::Respond`].
    pub fn respond(id: u32, timeout: Duration, reply_id: u32, reply_payload: Vec<u8>) -> Self {
        Self::Respond {
            id,
            timeout,
            reply_id,
            reply_payload,
        }
    }

    /// A quiet-drain step; see [`Step::Quiet`].
    pub fn quiet(quiet: Duration, limit: Duration) -> Self {
        Self::Quiet { quiet, limit }
    }
}

/// Errors of the test client.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// The TCP connection could not be established.
    #[error("cannot connect to {addr}: {source}")]
    Connect {
        /// Server address.
        addr: SocketAddr,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Reading or writing failed or timed out.
    #[error("i/o in state {state:?}: {source}")]
    Io {
        /// Protocol state when it happened.
        state: ProtocolState,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A step ran out of time.
    #[error("timeout in state {state:?} while {what}")]
    Timeout {
        /// Protocol state.
        state: ProtocolState,
        /// What was being awaited.
        what: String,
    },
    /// The server closed the connection .
    #[error("server closed the connection in state {state:?}")]
    Closed {
        /// Protocol state.
        state: ProtocolState,
    },
    /// The wire bytes could not be decoded (VarInt, framing, compression).
    #[error("malformed packet in state {state:?}: {source}")]
    Malformed {
        /// Protocol state.
        state: ProtocolState,
        /// Underlying codec error.
        source: protocol::CodecError,
    },
    /// A compressed frame could not be inflated or a frame was too large.
    #[error("frame error in state {state:?}: {reason}")]
    Frame {
        /// Protocol state.
        state: ProtocolState,
        /// Human-readable cause.
        reason: String,
    },
}

/// A scripted connection to one server that records a [`Trace`].
#[derive(Debug)]
pub struct Client {
    /// The socket, read through a buffer so one frame costs one syscall
    /// instead of one per length byte and one per body (ЧАСТЬ II: the bot
    /// must not be the slow side by accident).
    stream: BufReader<TcpStream>,
    state: ProtocolState,
    /// `None` until vanilla sends Set Compression (or it is disabled).
    threshold: Option<i32>,
    ids: Arc<PacketIds>,
    /// Reverse map for readable names in the trace: (state, direction, id).
    ///
    /// Built from the packets report when one is configured (every packet
    /// of the recording gets a name, which both reports and the diff's id
    /// normalisation rely on); without a report only the compiled packet
    /// keys are labelled.
    names: HashMap<(u8, u8, u32), String>,
    trace: Trace,
    /// Packets already satisfied for wait steps: waits check the recorded
    /// tail first, so packets read by a previous (possibly timed-out) step
    /// are never lost or read twice.
    read_pos: usize,
    /// When the connection attempt started: the base of every
    /// [`Self::arrivals`] timestamp (captured before the TCP connect, so
    /// the span being measured covers the connect itself).
    opened: std::time::Instant,
    /// Elapsed [`Self::opened`] when each packet arrived: `(id, elapsed)`
    /// of the packets received so far, in arrival order. Deliberately kept
    /// out of the [`Trace`]: a timestamp in the JSON would break byte and
    /// field comparisons against vanilla, while a test can still ask how
    /// long taking the recording took (part A 2, first and last chunk).
    arrivals: Vec<(u32, Duration)>,
    /// Elapsed [`Self::opened`] for **every** recorded packet (both
    /// directions): `stamps[i]` is the wire time of `trace.packets[i]`.
    ///
    /// Captured immediately after the frame left the socket (or arrived on
    /// it), never when the packet is later processed — a trace line keeps
    /// its real moment even when packets are emitted in a burst long after
    /// the receipt (the walk bot's JSONL trace).
    stamps: Vec<Duration>,
}

impl Client {
    /// Connects and prepares a trace labelled with `target`.
    pub async fn connect(
        target: &str,
        addr: SocketAddr,
        ids: Arc<PacketIds>,
    ) -> Result<Self, ClientError> {
        let opened = std::time::Instant::now();
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|source| ClientError::Connect { addr, source })?;
        stream.set_nodelay(true).map_err(|source| ClientError::Io {
            state: ProtocolState::Handshake,
            source,
        })?;
        let stream = BufReader::new(stream);

        // A report gives names to every traceable packet; without one only
        // the compiled keys are known (the trace then keeps raw ids).
        let names = if ids.is_empty() {
            protocol::used_packet_keys()
                .iter()
                .map(|key| {
                    (
                        (
                            key.state.index() as u8,
                            key.direction.index() as u8,
                            ids.id(*key),
                        ),
                        (*key.name).to_owned(),
                    )
                })
                .collect()
        } else {
            ids.names_by_id()
        };

        Ok(Self {
            stream,
            state: ProtocolState::Handshake,
            threshold: None,
            ids,
            names,
            trace: Trace::new(target),
            read_pos: 0,
            opened,
            arrivals: Vec::new(),
            stamps: Vec::new(),
        })
    }

    /// The moment each packet arrived: `(packet id, elapsed since the
    /// connection attempt started)`, in arrival order.
    pub fn arrivals(&self) -> &[(u32, Duration)] {
        &self.arrivals
    }

    /// Monotonic time of the socket read/write of every recorded packet:
    /// elapsed since the connection attempt, one per packet, in trace
    /// order (`stamps()[i]` belongs to `trace().packets[i]`).
    pub fn stamps(&self) -> &[Duration] {
        &self.stamps
    }

    /// Runs the whole script in order.
    pub async fn run(&mut self, steps: &[Step]) -> Result<(), ClientError> {
        for step in steps {
            match step {
                Step::Send { id, payload, to } => {
                    self.send(*id, payload).await?;
                    if let Some(state) = to {
                        self.state = *state;
                    }
                }
                Step::Wait { id, timeout } => self.wait_for(*id, *timeout).await?,
                Step::Respond {
                    id,
                    timeout,
                    reply_id,
                    reply_payload,
                } => {
                    self.respond(*id, *timeout, *reply_id, reply_payload)
                        .await?
                }
                Step::Quiet { quiet, limit } => {
                    self.drain(*quiet, *limit).await?;
                    // The drain deliberately read everything up to silence.
                    self.read_pos = self.trace.len();
                }
            }
        }
        Ok(())
    }

    /// Sends one packet by numeric id and records it (the walk bot's
    /// play loop needs plain sends outside the step script).
    ///
    /// The packet is framed exactly like a scripted `Step::Send`.
    pub async fn send_packet(&mut self, id: u32, payload: &[u8]) -> Result<(), ClientError> {
        self.send(id, payload).await
    }

    /// Reads one packet until `until` (`None` = the deadline passed,
    /// exactly like the internal waits see it). Play keep-alives are
    /// acknowledged automatically on the way in.
    ///
    /// Used by the walk bot, which must both read continuously and keep
    /// its own 50 ms send cadence.
    pub async fn recv_packet(
        &mut self,
        until: tokio::time::Instant,
    ) -> Result<Option<(u32, Vec<u8>)>, ClientError> {
        self.recv_until(until).await
    }

    /// The most recently recorded packet, if any.
    pub fn last_entry(&self) -> Option<&TraceEntry> {
        self.trace.packets.last()
    }

    /// The finished recording.
    pub fn into_trace(self) -> Trace {
        self.trace
    }

    /// The recording so far.
    pub fn trace(&self) -> &Trace {
        &self.trace
    }

    /// Sends one packet and records it.
    async fn send(&mut self, id: u32, payload: &[u8]) -> Result<(), ClientError> {
        let frame = self.encode_frame(id, payload)?;
        self.stream
            .write_all(&frame)
            .await
            .map_err(|source| ClientError::Io {
                state: self.state,
                source,
            })?;
        self.stream
            .flush()
            .await
            .map_err(|source| ClientError::Io {
                state: self.state,
                source,
            })?;
        // The stamp belongs to the moment the packet left the wire.
        let stamp = self.opened.elapsed();
        self.record(Direction::Serverbound, id, payload, stamp);
        Ok(())
    }

    /// Records one packet with the wire time it was read/written at and
    /// its resolved name: `stamps` and `trace.packets` grow together, so
    /// index `i` addresses the same packet in both.
    fn record(&mut self, direction: Direction, id: u32, payload: &[u8], stamp: Duration) {
        self.trace.record(self.state, direction, id, payload);
        self.stamps.push(stamp);
        let key = (self.state.index() as u8, direction.index() as u8, id);
        if let Some(name) = self.names.get(&key) {
            if let Some(entry) = self.trace.packets.last_mut() {
                entry.name = Some(name.clone());
            }
        }
    }

    /// Finds a recorded clientbound packet `id` of the current state that no
    /// earlier wait step has consumed yet.
    fn find_since(&self, id: u32) -> Option<usize> {
        let state = self.state.into();
        self.trace
            .packets
            .iter()
            .enumerate()
            .skip(self.read_pos)
            .find(|(_, entry)| {
                entry.state == state
                    && entry.direction == crate::trace::TraceDirection::Clientbound
                    && entry.id == id
            })
            .map(|(index, _)| index)
    }

    /// Waits until a clientbound packet with `id` arrives, or finds one a
    /// previous step already recorded but did not consume.
    async fn wait_for(&mut self, id: u32, timeout: Duration) -> Result<(), ClientError> {
        if let Some(index) = self.find_since(id) {
            self.read_pos = index + 1;
            return Ok(());
        }
        let until = Instant::now() + timeout;
        loop {
            match self.recv_until(until).await? {
                Some((got, _)) if got == id => {
                    self.read_pos = self.trace.len();
                    return Ok(());
                }
                Some(_) => {}
                None => {
                    return Err(ClientError::Timeout {
                        state: self.state,
                        what: format!("packet 0x{id:X}"),
                    });
                }
            }
        }
    }

    /// Answers a server probe when it arrives within `timeout`, and does
    /// nothing (quietly continuing the script) when it never does.
    async fn respond(
        &mut self,
        id: u32,
        timeout: Duration,
        reply_id: u32,
        reply_payload: &[u8],
    ) -> Result<(), ClientError> {
        if let Some(index) = self.find_since(id) {
            self.read_pos = index + 1;
            return self.send(reply_id, reply_payload).await;
        }
        let until = Instant::now() + timeout;
        loop {
            match self.recv_until(until).await? {
                Some((got, _)) if got == id => {
                    self.read_pos = self.trace.len();
                    return self.send(reply_id, reply_payload).await;
                }
                Some(_) => {}
                // The server performs no such exchange: move on.
                None => return Ok(()),
            }
        }
    }

    /// Reads until the line is quiet for `quiet`, capped at `limit`.
    async fn drain(&mut self, quiet: Duration, limit: Duration) -> Result<(), ClientError> {
        let start = Instant::now();
        let deadline = start + limit;
        let mut last_packet = start;
        loop {
            let quiet_deadline = last_packet + quiet;
            if quiet_deadline >= deadline {
                return Ok(());
            }
            match self.recv_until(quiet_deadline).await? {
                Some(_) => last_packet = Instant::now(),
                None => return Ok(()),
            }
        }
    }

    /// Receives one packet before `until`; `None` means the deadline passed.
    async fn recv_until(&mut self, until: Instant) -> Result<Option<(u32, Vec<u8>)>, ClientError> {
        let body =
            match tokio::time::timeout_at(until, read_varint_prefixed(&mut self.stream)).await {
                Ok(Ok(body)) => body,
                Ok(Err(source)) => {
                    if source.kind() == std::io::ErrorKind::UnexpectedEof {
                        return Err(ClientError::Closed { state: self.state });
                    }
                    return Err(ClientError::Io {
                        state: self.state,
                        source,
                    });
                }
                Err(_) => return Ok(None),
            };

        // Taken at the socket, before decompression: a heavy chunk's
        // timestamp is the moment its bytes hit the wire, not the moment
        // the payload was inflated.
        let stamp = self.opened.elapsed();
        let state = self.state;
        let body = match self.threshold {
            None => body,
            Some(_) => self.decompress(&body)?,
        };
        let (id, payload) =
            split_packet(&body).map_err(|source| ClientError::Malformed { state, source })?;

        self.record(Direction::Clientbound, id, &payload, stamp);
        self.arrivals.push((id, stamp));
        self.apply_set_compression(id, &payload)?;
        // Transport level, like compression: a real client acknowledges
        // play keep-alives, and without an acknowledgement vanilla drops an
        // idle recording after its ~30 second timeout.
        self.ack_keep_alive(id, &payload).await?;
        Ok(Some((id, payload)))
    }

    /// Inflates a compressed packet body after its DataLength VarInt.
    fn decompress(&self, body: &[u8]) -> Result<Vec<u8>, ClientError> {
        let state = self.state;
        let (data_len, consumed) = protocol::decode_varint(body)
            .map_err(|source| ClientError::Malformed { state, source })?;
        let rest = body.get(consumed..).ok_or_else(|| ClientError::Frame {
            state,
            reason: "truncated DataLength".to_owned(),
        })?;
        if data_len == 0 {
            // Below the threshold the client sent the packet uncompressed.
            return Ok(rest.to_vec());
        }
        let expected = usize::try_from(data_len).unwrap_or(usize::MAX);
        if expected > MAX_BODY {
            return Err(ClientError::Frame {
                state,
                reason: format!("declared uncompressed length {expected}"),
            });
        }
        let mut decoder = ZlibDecoder::new(rest);
        let mut out = Vec::with_capacity(expected);
        std::io::Read::read_to_end(&mut decoder, &mut out).map_err(|source| {
            ClientError::Frame {
                state,
                reason: format!("inflate failed: {source}"),
            }
        })?;
        if out.len() != expected {
            return Err(ClientError::Frame {
                state,
                reason: format!("declared {expected} uncompressed bytes, got {}", out.len()),
            });
        }
        Ok(out)
    }

    /// Turns on compression when Set Compression arrives (transport only).
    fn apply_set_compression(&mut self, id: u32, payload: &[u8]) -> Result<(), ClientError> {
        if self.threshold.is_some() || self.state != ProtocolState::Login {
            return Ok(());
        }
        if id != self.ids.id(SetCompression::KEY) {
            return Ok(());
        }
        let (threshold, _) =
            protocol::decode_varint(payload).map_err(|source| ClientError::Malformed {
                state: self.state,
                source,
            })?;
        // A negative threshold means "compression off" in the protocol.
        self.threshold = if threshold < 0 { None } else { Some(threshold) };
        Ok(())
    }

    /// Echoes a play Keep Alive back as the serverbound Keep Alive.
    ///
    /// Only the play state is covered: the scripts stay fast enough in the
    /// other states for vanilla to keep the connection open on its own.
    async fn ack_keep_alive(&mut self, id: u32, payload: &[u8]) -> Result<(), ClientError> {
        if self.state != ProtocolState::Play || id != self.ids.id(KeepAlive::KEY) {
            return Ok(());
        }
        let reply_id = self.ids.id(KeepAlive::SERVERBOUND_KEY);
        self.send(reply_id, payload).await
    }

    /// Builds one wire frame, compressing it when vanilla's threshold is on.
    fn encode_frame(&self, id: u32, payload: &[u8]) -> Result<Vec<u8>, ClientError> {
        build_frame(self.threshold, self.state, id, payload)
    }
}

/// Builds a complete frame for `id`/`payload` under an optional compression
/// threshold (the same rules the vanilla packet streams apply).
fn build_frame(
    threshold: Option<i32>,
    state: ProtocolState,
    id: u32,
    payload: &[u8],
) -> Result<Vec<u8>, ClientError> {
    let Some(threshold) = threshold.filter(|&value| value >= 0) else {
        return protocol::encode_frame(id, payload)
            .map_err(|source| ClientError::Malformed { state, source });
    };

    let id_i32 = i32::try_from(id).map_err(|_| ClientError::Frame {
        state,
        reason: format!("packet id {id} does not fit in a VarInt"),
    })?;
    let mut raw = Vec::new();
    encode_varint(id_i32, &mut raw);
    raw.extend_from_slice(payload);

    let threshold = usize::try_from(threshold).unwrap_or(usize::MAX);
    let framed = if raw.len() < threshold {
        // Below the threshold: DataLength = 0, bytes stay raw.
        let mut out = Vec::new();
        encode_varint(0, &mut out);
        out.extend_from_slice(&raw);
        out
    } else {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut encoder, &raw).map_err(|source| ClientError::Frame {
            state,
            reason: format!("deflate failed: {source}"),
        })?;
        let compressed = encoder.finish().map_err(|source| ClientError::Frame {
            state,
            reason: format!("deflate finish failed: {source}"),
        })?;
        let mut out = Vec::new();
        encode_varint(raw.len() as i32, &mut out);
        out.extend_from_slice(&compressed);
        out
    };

    let mut frame = Vec::new();
    encode_varint(framed.len() as i32, &mut frame);
    frame.extend_from_slice(&framed);
    Ok(frame)
}

/// Reads one VarInt-prefixed frame body off the stream.
async fn read_varint_prefixed<R: tokio::io::AsyncRead + Unpin>(
    stream: &mut R,
) -> std::io::Result<Vec<u8>> {
    let mut length = 0usize;
    let mut shift = 0;
    loop {
        let mut byte = [0u8; 1];
        stream.read_exact(&mut byte).await?;
        length |= usize::from(byte[0] & 0x7F) << shift;
        if byte[0] & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 35 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "frame length VarInt too long",
            ));
        }
    }
    if length > MAX_BODY {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame body too large: {length}"),
        ));
    }
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).await?;
    Ok(body)
}

/// Splits `packet id VarInt + payload` into its parts.
fn split_packet(body: &[u8]) -> Result<(u32, Vec<u8>), protocol::CodecError> {
    let (id, consumed) = decode_varint(body)?;
    let id = u32::try_from(id).map_err(|_| protocol::CodecError::InvalidPacketId(id))?;
    Ok((id, body[consumed..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATE: ProtocolState = ProtocolState::Login;

    /// Reads one varint-prefixed frame body off a socket (fake server side).
    async fn read_frame(sock: &mut tokio::net::TcpStream) -> Vec<u8> {
        use tokio::io::AsyncReadExt;
        let mut length = 0usize;
        let mut shift = 0;
        loop {
            let mut byte = [0u8; 1];
            sock.read_exact(&mut byte).await.expect("length byte");
            length |= usize::from(byte[0] & 0x7F) << shift;
            if byte[0] & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        let mut body = vec![0u8; length];
        sock.read_exact(&mut body).await.expect("frame body");
        body
    }

    /// A play Keep Alive must be answered with the same payload so vanilla
    /// does not time an idle recording out.
    #[tokio::test]
    async fn play_keep_alive_is_acknowledged() {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("local addr");
        let ids = Arc::new(PacketIds::compiled());
        let server_ids = Arc::clone(&ids);
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.expect("accept");
            // The script's first step: one handshake frame.
            read_frame(&mut sock).await;
            let payload = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
            let frame =
                protocol::encode_frame(server_ids.id(KeepAlive::KEY), &payload).expect("frame");
            sock.write_all(&frame).await.expect("keep-alive written");
            let ack = read_frame(&mut sock).await;
            let (id, consumed) = protocol::decode_varint(&ack).expect("ack id");
            assert_eq!(
                id,
                i32::try_from(server_ids.id(KeepAlive::SERVERBOUND_KEY)).expect("fits i32")
            );
            assert_eq!(&ack[consumed..], payload, "ack echoes the payload");
            // Hold the connection open until the client's drain ends: an
            // EOF on the wire is a lost connection, not a normal ending.
            let _ = done_rx.await;
        });

        let mut client = Client::connect("loopback", addr, Arc::clone(&ids))
            .await
            .expect("connect");
        let steps = [
            Step::send(0x00, Vec::new(), Some(ProtocolState::Play)),
            Step::quiet(Duration::from_millis(500), Duration::from_secs(5)),
        ];
        client.run(&steps).await.expect("script runs");
        drop(done_tx);
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .expect("fake server finishes")
            .expect("fake server task");
    }

    #[test]
    fn split_packet_reads_id_then_payload() {
        let (id, payload) = split_packet(&[0x00, 0xAA, 0xBB]).expect("well formed");
        assert_eq!(id, 0x00);
        assert_eq!(payload, [0xAA, 0xBB]);
    }

    #[test]
    fn without_threshold_the_frame_is_the_plain_protocol_frame() {
        let expected = protocol::encode_frame(0x00, &[1, 2]).expect("encodable");
        let frame = build_frame(None, STATE, 0x00, &[1, 2]).expect("frame");
        assert_eq!(frame, expected);
    }

    #[test]
    fn short_packets_below_the_threshold_stay_uncompressed() {
        let frame = build_frame(Some(256), STATE, 0x03, &[]).expect("frame");
        // Length + DataLength(0) + packet id.
        assert_eq!(frame, vec![0x02, 0x00, 0x03]);

        // Round trip through the receiving path: DataLength 0 = raw bytes.
        let body = frame[1..].to_vec();
        let (data_len, consumed) = decode_varint(&body).expect("data length");
        assert_eq!(data_len, 0);
        let (id, payload) = split_packet(&body[consumed..]).expect("packet");
        assert_eq!(id, 0x03);
        assert!(payload.is_empty());
    }

    #[test]
    fn packets_at_the_threshold_are_zlib_compressed() {
        let payload = vec![0xAB; 600];
        let frame = build_frame(Some(256), STATE, 0x07, &payload).expect("frame");

        let (frame_len, frame_consumed) = decode_varint(&frame).expect("frame length");
        assert_eq!(
            usize::try_from(frame_len).expect("positive"),
            frame.len() - frame_consumed
        );

        let body = &frame[frame_consumed..];
        let (data_len, body_consumed) = decode_varint(body).expect("data length");
        assert_eq!(usize::try_from(data_len).expect("positive"), 601);

        let mut decoder = ZlibDecoder::new(&body[body_consumed..]);
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut decoder, &mut raw).expect("inflate");
        let (id, inflated_payload) = split_packet(&raw).expect("packet");
        assert_eq!(id, 0x07);
        assert_eq!(inflated_payload, payload.as_slice());
    }
}
