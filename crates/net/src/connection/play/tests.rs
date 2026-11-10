//! Integration-shaped tests of the play loop: the join order, the
//! keep-alive cadence and the framing of a chunk burst.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use protocol::{
    ChunkBatchFinished, ChunkBatchStart, ForgetLevelChunk, GameEvent, LevelChunkWithLight,
    LoginPlay, PacketKey, PlayerInfoUpdate, SetCenterChunk, SetDefaultSpawnPosition,
    SynchronizePlayerPosition,
};
use tokio::io::{AsyncWriteExt, BufReader};

use super::*;
use crate::chunks::ReadyChunks;
use crate::frame::read_frame;

/// A game thread for a test. The thread is detached and ends when its
/// handle is dropped at the end of the test.
fn test_game() -> GameHandle {
    GameHandle::spawn(crate::game::GameConfig::default())
        .expect("spawn the game thread")
        .1
}

/// World source whose read parks at a barrier: the pool worker sits in
/// the "disk read" until the test releases it.
struct GatedSource {
    /// Two parties: the loading pool worker and the releasing test.
    release: Arc<std::sync::Barrier>,
}

impl crate::loader::ChunkSource for GatedSource {
    fn load_batch(&self) -> Result<Arc<ReadyChunks>, LoadError> {
        // This is the whole point of the pool: the slow world read
        // parks on a pool worker thread, not on the connection.
        self.release.wait();
        Ok(Arc::new(ReadyChunks::new(vec![one_payload()])))
    }
}

/// One tiny well-formed chunk payload for the fixture source.
fn one_payload() -> LevelChunkWithLight {
    LevelChunkWithLight {
        chunk_x: 6,
        chunk_z: -2,
        heightmaps: vec![protocol::HeightmapRecord {
            id: 4,
            words: vec![0],
        }],
        sections: vec![protocol::ChunkSection {
            non_air: 0,
            fluid: 0,
            blocks: protocol::PaletteContainer::Single(0),
            biomes: protocol::PaletteContainer::Single(1),
        }],
        block_entities: Vec::new(),
        light: protocol::LightData {
            sky_mask: vec![0x00],
            block_mask: vec![0x00],
            empty_sky_mask: vec![0x01],
            empty_block_mask: vec![0x01],
            sky_arrays: Vec::new(),
            block_arrays: Vec::new(),
        },
    }
}

/// World source serving one fixed fixture batch: the play loop gets the
/// same chunks on every read without touching a disk.
struct FixedSource {
    /// The ready world, shared between the pool worker and the test.
    chunks: Arc<ReadyChunks>,
}

impl crate::loader::ChunkSource for FixedSource {
    fn load_batch(&self) -> Result<Arc<ReadyChunks>, LoadError> {
        Ok(Arc::clone(&self.chunks))
    }
}

/// Reads the next frame from the client side and splits id and payload.
async fn next_packet<R>(reader: &mut R) -> (u32, Vec<u8>)
where
    R: AsyncRead + Unpin,
{
    try_next_packet(reader)
        .await
        .expect("frame from the server")
}

/// [`next_packet`] without the expectation: an error is the end of the
/// stream (a closed or broken connection), which some tests assert on.
async fn try_next_packet<R>(reader: &mut R) -> Result<(u32, Vec<u8>), String>
where
    R: AsyncRead + Unpin,
{
    let body = read_frame(reader).await.map_err(|err| err.to_string())?;
    let mut split = protocol::PacketReader::new(&body);
    let id = split.read_varint().map_err(|err| err.to_string())? as u32;
    let payload = split
        .read_bytes(split.remaining())
        .map_err(|err| err.to_string())?;
    Ok((id, payload.to_vec()))
}

/// Reads one whole sub-batch (`chunk_batch_start` ... `finished`) from
/// `reader` and returns how many chunk payloads sat in between.
async fn read_flush<R>(reader: &mut R, ids: &PacketIds) -> i32
where
    R: AsyncRead + Unpin,
{
    let (id, _) = next_packet(reader).await;
    assert_eq!(
        id,
        ids.id(ChunkBatchStart::KEY),
        "a flush opens with its start marker"
    );
    let mut count = 0_i32;
    loop {
        let (id, payload) = next_packet(reader).await;
        if id == ids.id(LevelChunkWithLight::KEY) {
            count += 1;
            continue;
        }
        assert_eq!(
            id,
            ids.id(ChunkBatchFinished::KEY),
            "a flush closes with finished"
        );
        let finished =
            ChunkBatchFinished::decode_payload(&mut protocol::PacketReader::new(&payload))
                .expect("decodable");
        assert_eq!(
            finished.chunk_count, count,
            "the finished count equals the payloads in between"
        );
        return count;
    }
}

/// The status document every test connection is started with.
fn test_status() -> StatusInfo {
    StatusInfo {
        version_name: "test".to_owned(),
        protocol_version: 1,
        max_players: 1,
        online_players: 0,
        motd: "test".to_owned(),
    }
}

/// The join sequence every play connection sends first.
fn join_keys() -> [PacketKey; 6] {
    [
        LoginPlay::KEY,
        SetDefaultSpawnPosition::KEY,
        PlayerInfoUpdate::KEY,
        SetCenterChunk::KEY,
        GameEvent::KEY,
        SynchronizePlayerPosition::KEY,
    ]
}

/// The property of step 3.3c: while the loader pool is stuck inside the
/// world read, the play loop still sends the keep-alive on its tick —
/// and the chunk batch follows only once the read is released.
///
/// Time is paused: without a released read the only thing that can fire
/// is the keep-alive interval, which is exactly what is being asserted.
#[tokio::test]
async fn reading_the_world_never_defers_the_keep_alive() {
    tokio::time::pause();
    let ids = Arc::new(PacketIds::compiled());
    let release = Arc::new(std::sync::Barrier::new(2));
    let loader = Loader::spawn(Arc::new(GatedSource {
        release: Arc::clone(&release),
    }))
    .expect("pool threads");

    let (client, server) = tokio::io::duplex(1 << 20);
    let (server_read, server_write) = tokio::io::split(server);
    let ids_in_task = Arc::clone(&ids);
    let joined = tokio::spawn(async move {
        let game_data = GameData::default();
        let status = test_status();
        let client = Client {
            player_name: "Steve".to_owned(),
            view_distance: Some(8),
        };
        play_state(
            server_read,
            server_write,
            Session {
                game_data: &game_data,
                status: &status,
                client: &client,
                ids: &ids_in_task,
                world: World {
                    loader: Some(loader),
                    info: WorldInfo::default(),
                },
            },
            test_game(),
        )
        .await
    });

    let mut client = BufReader::new(client);
    // The join sequence arrives first, unchanged by the chunk path.
    for key in join_keys() {
        let (id, _) = next_packet(&mut client).await;
        assert_eq!(id, ids.id(key), "join sequence packet");
    }

    // The world read is still parked at the barrier: the next packet on
    // the wire has to be the keep-alive of the loop (its tick at 15 s
    // under paused time; the timeout below is later and never wins).
    let (id, _) = tokio::time::timeout(Duration::from_secs(20), next_packet(&mut client))
        .await
        .expect("the keep-alive must not wait for the world read");
    assert_eq!(id, ids.id(KeepAlive::KEY), "keep alive was deferred");

    // The property is established; put the clock back to real time.
    // While paused, an idle runtime jumps to the next keep-alive tick
    // again and again while the pool is still building the batch, which
    // would flood the stream with keep-alives here — with real time the
    // next tick is a whole interval away.
    tokio::time::resume();

    // Release the read: now the batch may go out, in the recorded
    // framing — start marker, the payload, finished with its count.
    release.wait();
    let (id, _) = next_packet(&mut client).await;
    assert_eq!(id, ids.id(ChunkBatchStart::KEY), "batch start");
    let (id, _) = next_packet(&mut client).await;
    assert_eq!(id, ids.id(LevelChunkWithLight::KEY), "the chunk");
    let (id, payload) = next_packet(&mut client).await;
    assert_eq!(id, ids.id(ChunkBatchFinished::KEY), "batch finished");
    let finished = ChunkBatchFinished::decode_payload(&mut protocol::PacketReader::new(&payload))
        .expect("decodable");
    assert_eq!(finished.chunk_count, 1, "one chunk in the batch");

    // The client goes away: play ends at once, either cleanly or with the
    // classified disconnect (the dropped duplex reads as EOF).
    drop(client);
    let result = tokio::time::timeout(Duration::from_secs(5), joined)
        .await
        .expect("play state must finish")
        .expect("play task must not panic");
    match result {
        Ok(()) => {}
        Err(err) if err.is_client_disconnect() => {}
        other => panic!("play ended with {other:?}"),
    }
}

/// Step 3.3d, pure pacing: once the client wants one chunk per tick, a
/// burst of three leaves as three own sub-batches — each framed exactly
/// as the recording frames a whole burst (start, payload, finished=1) —
/// and at least one game tick apart. Real time: the sleep of the pace is
/// the very thing under test.
#[tokio::test]
async fn a_confirmed_pace_splits_a_burst_into_tick_sized_sub_batches() {
    let ids = Arc::new(PacketIds::compiled());
    let (client, server) = tokio::io::duplex(1 << 20);
    let (_server_read, mut server_write) = tokio::io::split(server);
    let ids_in_task = Arc::clone(&ids);
    let started = std::time::Instant::now();
    let written = tokio::spawn(async move {
        let mut pace = ChunkPace::default();
        pace.ack(1.0);
        let chunks: Vec<_> = (0..3)
            .map(|index| LevelChunkWithLight {
                chunk_x: index,
                ..one_payload()
            })
            .collect();
        // Centre 0,0 keeps every chunk of this fixture in view: radius 10
        // covers x 0..2 and z -2 without a single chunk being filtered out.
        write_chunk_batch(
            &mut server_write,
            &ids_in_task,
            Arc::new(ReadyChunks::new(chunks)),
            &pace,
            0,
            0,
            10,
        )
        .await
    });

    let mut client = BufReader::new(client);
    for sub_batch in 0..3 {
        let (id, _) = next_packet(&mut client).await;
        assert_eq!(
            id,
            ids.id(ChunkBatchStart::KEY),
            "start of sub-batch {sub_batch}"
        );
        let (id, _) = next_packet(&mut client).await;
        assert_eq!(
            id,
            ids.id(LevelChunkWithLight::KEY),
            "the chunk of sub-batch {sub_batch}"
        );
        let (id, payload) = next_packet(&mut client).await;
        assert_eq!(
            id,
            ids.id(ChunkBatchFinished::KEY),
            "finished of sub-batch {sub_batch}"
        );
        let finished =
            ChunkBatchFinished::decode_payload(&mut protocol::PacketReader::new(&payload))
                .expect("decodable");
        assert_eq!(finished.chunk_count, 1, "one chunk per sub-batch");
    }
    tokio::time::timeout(Duration::from_secs(5), written)
        .await
        .expect("the burst must finish writing")
        .expect("write task must not panic")
        .expect("written without error");
    let passed = started.elapsed();
    assert!(
        passed >= Duration::from_millis(100),
        "two game ticks of pacing expected between three sub-batches, took {passed:?}"
    );
}

/// M4.4 A: chunks outside the join window of the connection never reach
/// the wire, and the batch framing counts only what was really sent — four
/// fixture chunks in, the two outside the radius-8 window of centre (0, 0)
/// dropped.
#[tokio::test]
async fn chunks_outside_the_send_radius_are_not_sent() {
    let ids = Arc::new(PacketIds::compiled());
    let (client, server) = tokio::io::duplex(1 << 20);
    let (_server_read, mut server_write) = tokio::io::split(server);
    let ids_in_task = Arc::clone(&ids);
    let written = tokio::spawn(async move {
        let pace = ChunkPace::default();
        // (0, -2) and (6, -2) inside the radius-8 window, (20, -2) and
        // (40, -2) out of it.
        let mut chunks: Vec<_> = [0, 20, 40]
            .into_iter()
            .map(|chunk_x| LevelChunkWithLight {
                chunk_x,
                ..one_payload()
            })
            .collect();
        chunks.push(one_payload());
        write_chunk_batch(
            &mut server_write,
            &ids_in_task,
            Arc::new(ReadyChunks::new(chunks)),
            &pace,
            0,
            0,
            8,
        )
        .await
    });

    let mut client = BufReader::new(client);
    let (id, _) = next_packet(&mut client).await;
    assert_eq!(id, ids.id(ChunkBatchStart::KEY), "batch start");
    let mut sent = 0;
    loop {
        let (id, payload) = next_packet(&mut client).await;
        if id == ids.id(LevelChunkWithLight::KEY) {
            sent += 1;
            continue;
        }
        assert_eq!(id, ids.id(ChunkBatchFinished::KEY), "batch finished");
        let finished =
            ChunkBatchFinished::decode_payload(&mut protocol::PacketReader::new(&payload))
                .expect("decodable");
        assert_eq!(sent, 2, "only the in-radius chunks");
        assert_eq!(
            finished.chunk_count, 2,
            "the finished count is filtered too"
        );
        break;
    }
    tokio::time::timeout(Duration::from_secs(5), written)
        .await
        .expect("the batch must finish writing")
        .expect("write task must not panic")
        .expect("written without error");
}

/// Part A, 2: the burst opens with the join centre and then every chunk
/// by non-decreasing squared distance — the order vanilla's flushes show
/// (its first flush is the 3x3 square around (6, -2)).
///
/// The fixture is the whole 17x17 square `self_session` records, handed
/// over in the row order it arrived in until part A 2: rows from the
/// corner (-2, -10), the same far corner the recording started with.
#[test]
fn the_burst_goes_out_from_the_centre_to_the_far_corners() {
    let square: Vec<LevelChunkWithLight> = (-10..=6)
        .flat_map(|chunk_z| (-2..=14).map(move |chunk_x| (chunk_x, chunk_z)))
        .map(|(chunk_x, chunk_z)| LevelChunkWithLight {
            chunk_x,
            chunk_z,
            ..one_payload()
        })
        .collect();
    let mut chunks: Vec<&LevelChunkWithLight> = square.iter().collect();

    sort_by_distance(&mut chunks, 6, -2);

    let order: Vec<(i32, i32)> = chunks
        .iter()
        .map(|chunk| (chunk.chunk_x, chunk.chunk_z))
        .collect();
    let squared = |(x, z): (i32, i32)| (x - 6).pow(2) + (z + 2).pow(2);
    assert_eq!(order.first().copied(), Some((6, -2)), "centre first");
    assert!(
        order
            .windows(2)
            .all(|pair| squared(pair[0]) <= squared(pair[1])),
        "the distance from the centre never falls: {order:?}"
    );
    assert!(
        order
            .windows(2)
            .all(|pair| squared(pair[0]) != squared(pair[1]) || pair[0] < pair[1]),
        "a distance tie is ordered by coordinates: {order:?}"
    );
    // The nine chunks vanilla opens with are exactly this square: the
    // centre, the four orthogonal neighbours and the four corners of 3x3.
    let mut first_nine = order[..9].to_vec();
    let mut vanilla = [
        (6, -2),
        (6, -1),
        (7, -2),
        (5, -2),
        (6, -3),
        (7, -1),
        (5, -1),
        (5, -3),
        (7, -3),
    ];
    first_nine.sort_unstable();
    vanilla.sort_unstable();
    assert_eq!(
        first_nine, vanilla,
        "the opening square of the vanilla flush"
    );
    assert_eq!(order.last().copied(), Some((14, 6)), "the far corner last");
}

/// Step 3.3d in the loop: the connection decodes the client's pace
/// answer and stays open without answering it (a composition the loop
/// cannot read would end the connection right here, and the next read
/// would return that error instead of silence).
#[tokio::test]
async fn the_pace_answer_keeps_the_connection_open() {
    let ids = Arc::new(PacketIds::compiled());
    let (client, server) = tokio::io::duplex(1 << 12);
    let (server_read, server_write) = tokio::io::split(server);
    let ids_in_task = Arc::clone(&ids);
    let joined = tokio::spawn(async move {
        let game_data = GameData::default();
        let status = test_status();
        let client = Client {
            player_name: "Steve".to_owned(),
            view_distance: None,
        };
        play_state(
            server_read,
            server_write,
            Session {
                game_data: &game_data,
                status: &status,
                client: &client,
                ids: &ids_in_task,
                world: World {
                    loader: None,
                    info: WorldInfo::default(),
                },
            },
            test_game(),
        )
        .await
    });

    let (client_read, mut client_write) = tokio::io::split(client);
    let mut client = BufReader::new(client_read);
    for key in join_keys() {
        let (id, _) = next_packet(&mut client).await;
        assert_eq!(id, ids.id(key), "join sequence packet");
    }

    // The pace answer of a real client, after the batch it reacts to.
    let ack = ChunkBatchReceived {
        chunks_per_tick: 8.0,
    };
    let frame = protocol::encode_frame(ids.id(ChunkBatchReceived::KEY), &ack.encode_payload())
        .expect("encodable pace answer");
    client_write.write_all(&frame).await.expect("sent");
    client_write.flush().await.expect("flushed");

    // A connection that cannot read the packet closes here; a healthy one
    // stays silent — the loop never answers a pace.
    let answered =
        tokio::time::timeout(Duration::from_millis(300), try_next_packet(&mut client)).await;
    match answered {
        Ok(Ok((id, _))) => panic!("unexpected server packet 0x{id:X} after the pace"),
        Ok(Err(err)) => panic!("the pace answer broke the connection: {err}"),
        Err(_) => {} // open and silent, exactly as it should be
    }

    drop(client);
    drop(client_write);
    let result = tokio::time::timeout(Duration::from_secs(5), joined)
        .await
        .expect("play state must finish")
        .expect("play task must not panic");
    match result {
        Ok(()) => {}
        Err(err) if err.is_client_disconnect() => {}
        other => panic!("play ended with {other:?}"),
    }
}

/// Part A 5: the join burst replays the vanilla trace's flush sequence on
/// our wire — 20 chunks open with a flush of exactly 9 (the 3x3 square
/// around the centre, centre first), the client's `chunk_batch_received`
/// (8.0, the number the recorded vanilla ack carries) paces the rest into
/// one flush of 8 and one of 3, the tail one game tick behind: `[9, 8, 3]`
/// over the same `[9, 8, 8, 3, ...]` rule of
/// `target/parity/chunk-batch-vanilla.json` on 72 chunks.
///
/// Real time: the speed of the waits (immediate behind the answer, one
/// tick before the last flush) is the very thing under test.
#[tokio::test]
async fn the_first_flush_is_nine_then_the_vanilla_paced_tail() {
    let ids = Arc::new(PacketIds::compiled());
    // Twenty chunks around the empty world's cache centre (0, 0): the
    // five-by-four cell x -2..2, z -1..2 inside radius 8.
    let chunks: Vec<LevelChunkWithLight> = (-2..=2)
        .flat_map(|chunk_x| (-1..=2).map(move |chunk_z| (chunk_x, chunk_z)))
        .map(|(chunk_x, chunk_z)| LevelChunkWithLight {
            chunk_x,
            chunk_z,
            ..one_payload()
        })
        .collect();
    assert_eq!(chunks.len(), 20, "the fixture batch");
    let loader = Loader::spawn(Arc::new(FixedSource {
        chunks: Arc::new(ReadyChunks::new(chunks)),
    }))
    .expect("pool threads");

    let (client, server) = tokio::io::duplex(1 << 20);
    let (client_read, mut client_write) = tokio::io::split(client);
    let (server_read, server_write) = tokio::io::split(server);
    let ids_in_task = Arc::clone(&ids);
    let joined = tokio::spawn(async move {
        let game_data = GameData::default();
        let status = test_status();
        let client = Client {
            player_name: "Steve".to_owned(),
            view_distance: Some(8),
        };
        play_state(
            server_read,
            server_write,
            Session {
                game_data: &game_data,
                status: &status,
                client: &client,
                ids: &ids_in_task,
                world: World {
                    loader: Some(loader),
                    info: WorldInfo::default(),
                },
            },
            test_game(),
        )
        .await
    });

    let mut client = BufReader::new(client_read);
    for key in join_keys() {
        let (id, _) = next_packet(&mut client).await;
        assert_eq!(id, ids.id(key), "join sequence packet");
    }

    // Flush 1: the opening of the recorded vanilla burst — nine chunks,
    // the 3x3 square the distance rule puts first.
    let first = read_flush(&mut client, &ids).await;
    assert_eq!(first, 9, "the first flush is the vanilla 3x3 square");

    // The answer a vanilla client sends right behind that first finished
    // marker: eight chunks per tick — this stops the guard wait and sizes
    // the tail.
    let ack = ChunkBatchReceived {
        chunks_per_tick: 8.0,
    };
    let frame = protocol::encode_frame(ids.id(ChunkBatchReceived::KEY), &ack.encode_payload())
        .expect("encodable pace answer");
    client_write.write_all(&frame).await.expect("sent");
    client_write.flush().await.expect("flushed");
    let answered = std::time::Instant::now();

    // Flush 2: right behind the answer, at the named rate — 11 chunks
    // left, eight go now.
    let second = read_flush(&mut client, &ids).await;
    assert_eq!(second, 8, "the tail starts at the client's own rate");
    let waited = answered.elapsed();
    assert!(
        waited < Duration::from_millis(500),
        "the answer must stop the guard wait at once, took {waited:?}"
    );

    // Flush 3: the last three, one game tick after the previous flush.
    let third = read_flush(&mut client, &ids).await;
    assert_eq!(third, 3, "the rest in one last flush");
    let gap = answered.elapsed();
    assert!(
        gap >= Duration::from_millis(50),
        "one game tick between the paced flushes, only {gap:?} since the answer"
    );
    assert_eq!(first + second + third, 20, "every fixture chunk went out");

    // The burst is complete: no fourth flush appears while the guard of
    // the pace answer would still be running.
    let extra =
        tokio::time::timeout(Duration::from_millis(300), try_next_packet(&mut client)).await;
    match extra {
        Ok(Ok((id, _))) => panic!("unexpected packet 0x{id:X} after the burst"),
        Ok(Err(err)) => panic!("the connection broke after the burst: {err}"),
        Err(_) => {} // silent, the whole burst is on the wire
    }

    drop(client);
    drop(client_write);
    let result = tokio::time::timeout(Duration::from_secs(5), joined)
        .await
        .expect("play state must finish")
        .expect("play task must not panic");
    match result {
        Ok(()) => {}
        Err(err) if err.is_client_disconnect() => {}
        other => panic!("play ended with {other:?}"),
    }
}

/// The ready world of the ШАГ 3 fixtures: a full square big enough for every
/// window a walk from centre (0, 0) east to (5, 0) touches at radius 8
/// (x −10..=14, z −10..=10), so nothing the walk needs is ever a hole.
fn walk_world() -> Arc<ReadyChunks> {
    let chunks: Vec<LevelChunkWithLight> = (-10..=14)
        .flat_map(|chunk_x| {
            (-10..=10).map(move |chunk_z| LevelChunkWithLight {
                chunk_x,
                chunk_z,
                ..one_payload()
            })
        })
        .collect();
    Arc::new(ReadyChunks::new(chunks))
}

/// Reads one whole sub-batch (`chunk_batch_start` ... `finished`) and returns
/// the coordinates of the chunks it carried.
async fn read_flush_coords<R>(reader: &mut R, ids: &PacketIds) -> Vec<(i32, i32)>
where
    R: AsyncRead + Unpin,
{
    let (id, _) = next_packet(reader).await;
    assert_eq!(id, ids.id(ChunkBatchStart::KEY), "a flush opens with start");
    let mut coords = Vec::new();
    loop {
        let (id, payload) = next_packet(reader).await;
        if id == ids.id(LevelChunkWithLight::KEY) {
            let chunk =
                LevelChunkWithLight::decode_payload(&mut protocol::PacketReader::new(&payload))
                    .expect("decodable chunk");
            coords.push((chunk.chunk_x, chunk.chunk_z));
            continue;
        }
        assert_eq!(
            id,
            ids.id(ChunkBatchFinished::KEY),
            "a flush closes with finished"
        );
        return coords;
    }
}

/// M4.4 B (the "faster than portions arrive" trap), sender side: the player
/// crosses five chunk boundaries while the queue still holds chunks of every
/// earlier window. The invariant — `sent ∪ queue` never leaves
/// `window(centre)` — must hold after every move; a leaving queued chunk is
/// dropped without a forget, a leaving sent chunk is forgotten; and the fully
/// drained sender ends at exactly `window(final centre) ∩ ready`.
#[tokio::test]
async fn a_fast_walk_never_leaves_a_chunk_in_an_old_window() {
    let ids = PacketIds::compiled();
    let ready = walk_world();
    let radius = 8;
    let mut pace = ChunkPace::default();
    // Two chunks a tick: the join window is nowhere near drained when the
    // walk starts, so every move really is "faster than the portions arrive".
    pace.ack(2.0);
    let mut batch = PendingBatch::new(Arc::clone(&ready), 0, 0, radius);
    let mut sink = tokio::io::sink();

    // One small portion of the join window; the rest stays queued across the
    // whole walk.
    batch
        .send_next(&mut sink, &ids, &pace)
        .await
        .expect("write the opening portion");
    assert!(!batch.is_idle(), "the join window is far from drained");
    assert!(batch.within_window(), "the opening portion is in-window");

    let mut center = (0, 0);
    for step in 1..=5_i32 {
        let next = (step, 0);
        let (add, remove) = crate::window_difference(center, next, radius);
        assert!(
            !add.is_empty() && !remove.is_empty(),
            "step {step} moves the window"
        );
        let sent_before = batch.sent().clone();
        let forget = batch.move_window(next, &add, &remove);

        // The invariant holds right after the move, before anything else.
        assert!(
            batch.within_window(),
            "step {step}: sent/queue left the new window"
        );
        // Every forget was a chunk already on the wire; a leaving chunk that
        // was still queued produces no forget at all.
        assert!(
            forget.iter().all(|cell| sent_before.contains(cell)),
            "step {step}: a queued chunk was forgotten"
        );
        assert!(
            forget.windows(2).all(|pair| pair[0] <= pair[1]),
            "step {step}: the forgets are not ascending (x, z)"
        );
        center = next;

        // Deliver one more small portion and re-check the invariant.
        if !batch.is_idle() {
            batch
                .send_next(&mut sink, &ids, &pace)
                .await
                .expect("write a portion");
            assert!(
                batch.within_window(),
                "step {step}: a portion left the window"
            );
        }
    }

    // Drain the rest; every write stays inside the final window.
    while !batch.is_idle() {
        batch
            .send_next(&mut sink, &ids, &pace)
            .await
            .expect("drain the queue");
        assert!(batch.within_window(), "the drain left the window");
    }

    let expected: BTreeSet<(i32, i32)> = crate::window(center.0, center.1, radius)
        .into_iter()
        .filter(|&(x, z)| ready.position(x, z).is_some())
        .collect();
    assert_eq!(
        batch.sent(),
        &expected,
        "the drained sender is exactly window({center:?}) ∩ ready"
    );
}

/// M4.4 B (wire order, ШАГ 3): a window move writes `set_chunk_cache_center`
/// of the new centre first, then `forget_level_chunk` for the leaving sent
/// chunks in ascending `(x, z)` (the traced order), and only the entering
/// chunks are left to send — nothing out of the old window follows the move.
#[tokio::test]
async fn a_window_move_writes_the_centre_then_the_forgets_in_order() {
    let ids = PacketIds::compiled();
    let ready = walk_world();
    let radius = 8;
    let mut pace = ChunkPace::default();
    pace.ack(1e9); // the whole join window in one piece
    let mut batch = PendingBatch::new(Arc::clone(&ready), 0, 0, radius);
    let mut sink = tokio::io::sink();
    assert!(
        batch
            .send_next(&mut sink, &ids, &pace)
            .await
            .expect("write the join window"),
        "the whole join window fits one sub-batch"
    );
    assert!(batch.is_idle(), "the join window is fully on the wire");
    let was_sent = batch.sent().clone();

    let (add, remove) = crate::window_difference((0, 0), (1, 0), radius);
    assert!(
        !add.is_empty() && !remove.is_empty(),
        "the window really moves"
    );
    assert!(
        remove.iter().all(|cell| was_sent.contains(cell)),
        "a fully sent window has every leaving chunk on the wire"
    );

    let (client, server) = tokio::io::duplex(1 << 20);
    let (_client_read, mut server_write) = tokio::io::split(server);
    let mut client = BufReader::new(client);

    let queued = write_window_move(&mut server_write, &ids, &mut batch, (1, 0), &add, &remove)
        .await
        .expect("write the window move");
    assert_eq!(queued, add.len(), "only the entering chunks were queued");

    // 1) the new centre goes first.
    let (id, payload) = next_packet(&mut client).await;
    assert_eq!(id, ids.id(SetCenterChunk::KEY), "the centre goes first");
    let center = SetCenterChunk::decode_payload(&mut protocol::PacketReader::new(&payload))
        .expect("decodable");
    assert_eq!(
        (center.chunk_x, center.chunk_z),
        (1, 0),
        "the announced centre is the new one"
    );

    // 2) then every leaving sent chunk as a forget, ascending (x, z).
    let mut forgotten = Vec::new();
    for _ in 0..remove.len() {
        let (id, payload) = next_packet(&mut client).await;
        assert_eq!(
            id,
            ids.id(ForgetLevelChunk::KEY),
            "a forget follows the centre"
        );
        let forget = ForgetLevelChunk::decode_payload(&mut protocol::PacketReader::new(&payload))
            .expect("decodable forget");
        forgotten.push((forget.chunk_x, forget.chunk_z));
    }
    assert_eq!(
        forgotten, remove,
        "the traced ascending (x, z) forget order"
    );

    // 3) the entering chunks wait for the batch arm; nothing old is resent.
    assert!(!batch.is_idle(), "the entering chunks are queued");
    assert!(batch
        .send_next(&mut server_write, &ids, &pace)
        .await
        .expect("write the entering chunks"));
    let sent_now = read_flush_coords(&mut client, &ids).await;
    let sent_set: BTreeSet<(i32, i32)> = sent_now.iter().copied().collect();
    let add_in_ready: BTreeSet<(i32, i32)> = add
        .iter()
        .copied()
        .filter(|&(x, z)| ready.position(x, z).is_some())
        .collect();
    assert_eq!(sent_set, add_in_ready, "only the entering chunks were sent");
    assert_eq!(
        sent_now.len(),
        sent_set.len(),
        "no chunk was sent twice in one flush"
    );
}
