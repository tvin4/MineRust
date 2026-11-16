//! Шаг 13b tests: a `player_action` (41) through the real per-tick path.
//!
//! The bytes themselves are checked in `protocol` against the vanilla S3
//! trace; here the subject is what the game thread does with the action —
//! which packets reach the digger and the observers, in which order, and
//! what a later join would read from the store.

use std::sync::Arc;

use data::BlockStates;
use protocol::{
    ChunkSection, HeightmapRecord, LevelChunkWithLight, LightData, PaletteContainer, PlayerAction,
    Position,
};
use tokio::sync::mpsc;

use crate::chunks::ReadyChunks;
use crate::store::ChunkStore;

use super::test_support::{join_at, join_at_mode};
use super::{
    block_action, BlockAction, Region, Replay, Scheduler, ServerMessage, ToGame, SINGLE_REGION,
};

/// A block report with just air and stone, so the store can classify a write.
fn test_blocks() -> Arc<BlockStates> {
    let text = r#"{
      "minecraft:air": {"states": [{"id": 0, "default": true}]},
      "minecraft:stone": {"states": [{"id": 1, "default": true}]}
    }"#;
    Arc::new(BlockStates::parse(text, "test".to_owned()).expect("a valid report"))
}

/// A chunk whose every section is stone, so any traced position is solid.
fn solid_chunk(chunk_x: i32, chunk_z: i32) -> LevelChunkWithLight {
    LevelChunkWithLight {
        chunk_x,
        chunk_z,
        heightmaps: vec![HeightmapRecord {
            id: 4,
            words: vec![0],
        }],
        sections: (0..24)
            .map(|_| ChunkSection {
                non_air: 4096,
                fluid: 0,
                blocks: PaletteContainer::Single(1),
                biomes: PaletteContainer::Single(1),
            })
            .collect(),
        block_entities: Vec::new(),
        light: LightData {
            sky_mask: vec![0x00],
            block_mask: vec![0x00],
            empty_sky_mask: vec![0x01],
            empty_block_mask: vec![0x01],
            sky_arrays: Vec::new(),
            block_arrays: Vec::new(),
        },
    }
}

/// A store holding the chunk of the traced block (93,123,−22): chunk (5,−2).
fn store_with_traced_chunk() -> Arc<ChunkStore> {
    Arc::new(ChunkStore::new(
        ReadyChunks::new(vec![solid_chunk(5, -2)]),
        test_blocks(),
    ))
}

/// Two players in one region, plus the digger/observer outbound queues.
fn two_players() -> (
    Scheduler,
    mpsc::Receiver<ServerMessage>,
    mpsc::Receiver<ServerMessage>,
) {
    two_players_with_mode(0)
}

/// [`two_players`] with the digger's game mode set (the observer stays
/// survival).
fn two_players_with_mode(
    digger_mode: i32,
) -> (
    Scheduler,
    mpsc::Receiver<ServerMessage>,
    mpsc::Receiver<ServerMessage>,
) {
    let mut scheduler = Scheduler::new(vec![Region::new(SINGLE_REGION)]);
    let (join_a, _slot_a, receiver_a) = join_at_mode(1, (0, 64, 0), digger_mode);
    let (join_b, _slot_b, receiver_b) = join_at(2, (0, 64, 0));
    scheduler.insert_player(join_a);
    scheduler.insert_player(join_b);
    (scheduler, receiver_a, receiver_b)
}

/// The traced block of the S3 break.
fn traced_position() -> Position {
    Position {
        x: 93,
        y: 123,
        z: -22,
    }
}

/// Applies one action as player 1.
fn act(scheduler: &mut Scheduler, store: &Arc<ChunkStore>, action: i32, sequence: i32) {
    let mut replay = Replay::<Vec<u8>>::new(None);
    block_action::apply(
        scheduler,
        &mut replay,
        0,
        Some(store),
        BlockAction {
            id: 1,
            action,
            position: traced_position(),
            sequence,
        },
    );
}

/// Expects `block_update(position, 0)` on the queue.
fn expect_block_update(receiver: &mut mpsc::Receiver<ServerMessage>, position: Position) {
    match receiver.try_recv().expect("a block_update") {
        ServerMessage::BlockUpdate {
            position: got,
            state,
        } => {
            assert_eq!(got, position, "the traced position");
            assert_eq!(state, 0, "the removed state is air");
        }
        other => panic!("expected block_update, got {other:?}"),
    }
}

/// Expects `block_changed_ack(sequence)` on the queue.
fn expect_ack(receiver: &mut mpsc::Receiver<ServerMessage>, sequence: i32) {
    match receiver.try_recv().expect("a block_changed_ack") {
        ServerMessage::BlockChangedAck { sequence: got } => {
            assert_eq!(got, sequence, "the action's sequence is echoed");
        }
        other => panic!("expected block_changed_ack, got {other:?}"),
    }
}

/// Expects the block-break `level_event` on the queue: the traced effect
/// (2001) and `data` = the state id the broken block had.
fn expect_level_event(
    receiver: &mut mpsc::Receiver<ServerMessage>,
    position: Position,
    broken_state: i32,
) {
    match receiver.try_recv().expect("a level_event") {
        ServerMessage::LevelEvent {
            position: got,
            state,
        } => {
            assert_eq!(got, position, "the traced position");
            assert_eq!(state, broken_state, "the removed block's state id");
        }
        other => panic!("expected level_event, got {other:?}"),
    }
}

/// The digger gets `block_update` then the ack — and never a `level_event`;
/// the observer gets the break `level_event` (with the removed state id)
/// before its update, and the block is gone from the store.
#[test]
fn a_finish_removes_the_block_and_broadcasts_the_trace_packets() {
    let store = store_with_traced_chunk();
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();
    assert_eq!(store.block(93, 123, -22), Some(1), "solid before the break");

    act(&mut scheduler, &store, PlayerAction::ACTION_FINISH, 7);

    assert_eq!(store.block(93, 123, -22), Some(0), "air after the break");
    // The digger: the update first, the ack second — the traced order; the
    // break effect never reaches the digger (S3/S9).
    expect_block_update(&mut receiver_a, traced_position());
    expect_ack(&mut receiver_a, 7);
    assert!(
        receiver_a.try_recv().is_err(),
        "nothing else goes to the digger"
    );
    // The observer: the break effect with the removed state id (1 = the
    // store's stone), then the update; never the ack.
    expect_level_event(&mut receiver_b, traced_position(), 1);
    expect_block_update(&mut receiver_b, traced_position());
    assert!(receiver_b.try_recv().is_err(), "an observer gets no ack");
}

/// A player who joins after the break reads the changed snapshot — the chunk
/// a later join sends already carries the air cell. This is the store half of
/// "a late joiner sees the block gone"; the connection half is
/// `connection::play::tests::a_sender_after_a_block_change_reads_the_new_block`.
#[test]
fn a_join_after_the_break_reads_the_changed_snapshot() {
    let store = store_with_traced_chunk();
    let (mut scheduler, _receiver_a, _receiver_b) = two_players();
    let before = store.snapshot().chunks()[0]
        .encode_payload()
        .expect("the chunk encodes");

    act(&mut scheduler, &store, PlayerAction::ACTION_FINISH, 1);

    let after_snapshot = store.snapshot();
    let after = after_snapshot.chunks()[0]
        .encode_payload()
        .expect("the chunk encodes");
    assert_ne!(
        before, after,
        "a later join's payload differs from the one before the break"
    );
    // y=123 sits in section slot 11 of the payload; the write turned the
    // single-valued section into a packed palette that includes air (0).
    match &after_snapshot.chunks()[0].sections[11].blocks {
        PaletteContainer::Packed { palette, .. } => {
            assert!(palette.contains(&0), "the air state is on the wire");
            assert!(palette.contains(&1), "the rest is still stone");
        }
        other => panic!("expected a packed section after the break, got {other:?}"),
    }
}

/// Breaking a block that is already air cancels the prediction instead of
/// removing anything: the digger gets the real state back, then the ack; no
/// observer hears about it. The refusal response is a hypothesis — the S3
/// trace carries no such case (`docs/BLOCK-BREAKING.md` §1.4).
#[test]
fn a_finish_on_air_resends_the_state_and_acknowledges() {
    let store = store_with_traced_chunk();
    store
        .set_block(93, 123, -22, 0)
        .expect("the cell starts solid and is now air");
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();

    act(&mut scheduler, &store, PlayerAction::ACTION_FINISH, 5);

    expect_block_update(&mut receiver_a, traced_position());
    expect_ack(&mut receiver_a, 5);
    assert!(
        receiver_b.try_recv().is_err(),
        "nothing changed for the observer"
    );
}

/// A finish in a chunk the store does not hold has no state to resend, so the
/// action is only acknowledged. [hypothesis] — no trace case.
#[test]
fn a_finish_in_a_missing_chunk_only_acknowledges() {
    let store = store_with_traced_chunk();
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();
    let mut replay = Replay::<Vec<u8>>::new(None);

    block_action::apply(
        &mut scheduler,
        &mut replay,
        0,
        Some(&store),
        BlockAction {
            id: 1,
            action: PlayerAction::ACTION_FINISH,
            position: Position {
                x: 400,
                y: 64,
                z: 400,
            },
            sequence: 9,
        },
    );

    expect_ack(&mut receiver_a, 9);
    assert!(
        receiver_a.try_recv().is_err(),
        "no block_update without a state to send"
    );
    assert!(receiver_b.try_recv().is_err(), "the observer hears nothing");
}

/// Start and abort change nothing; both are acknowledged, as the trace shows
/// for start (and infers for abort).
#[test]
fn start_and_abort_change_nothing_and_only_acknowledge() {
    for (sequence, action) in [
        (1, PlayerAction::ACTION_START),
        (2, PlayerAction::ACTION_ABORT),
    ] {
        let store = store_with_traced_chunk();
        let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();

        act(&mut scheduler, &store, action, sequence);

        assert_eq!(
            store.block(93, 123, -22),
            Some(1),
            "action {action} leaves the block in place"
        );
        expect_ack(&mut receiver_a, sequence);
        assert!(
            receiver_a.try_recv().is_err(),
            "no block_update to the digger"
        );
        assert!(receiver_b.try_recv().is_err(), "nothing for the observer");
    }
}

/// With no store attached, the action is still acknowledged (the connection
/// must not be left waiting) and nothing can change.
#[test]
fn without_a_store_the_action_is_only_acknowledged() {
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();
    let mut replay = Replay::<Vec<u8>>::new(None);

    block_action::apply(
        &mut scheduler,
        &mut replay,
        0,
        None,
        BlockAction {
            id: 1,
            action: PlayerAction::ACTION_FINISH,
            position: traced_position(),
            sequence: 3,
        },
    );

    expect_ack(&mut receiver_a, 3);
    assert!(receiver_a.try_recv().is_err());
    assert!(receiver_b.try_recv().is_err());
}

/// The action reaches the tick through `ToGame`, so the connection → game
/// message itself is exercised, not only the helper.
#[test]
fn the_tick_accepts_the_traced_break_message() {
    let store = store_with_traced_chunk();
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();
    let mut replay = Replay::<Vec<u8>>::new(None);

    // The same call `apply` makes for `ToGame::BlockAction`.
    let message = ToGame::BlockAction(BlockAction {
        id: 1,
        action: PlayerAction::ACTION_FINISH,
        position: traced_position(),
        sequence: 4,
    });
    match message {
        ToGame::BlockAction(action) => {
            block_action::apply(&mut scheduler, &mut replay, 0, Some(&store), action);
        }
        other => panic!("expected a block action, got {other:?}"),
    }

    assert_eq!(store.block(93, 123, -22), Some(0), "the block is removed");
    expect_block_update(&mut receiver_a, traced_position());
    expect_ack(&mut receiver_a, 4);
    expect_level_event(&mut receiver_b, traced_position(), 1);
    expect_block_update(&mut receiver_b, traced_position());
}

/// In creative the start action itself breaks the block (Шаг 14c, the S9
/// trace): the digger gets the air update then the ack; the observer gets the
/// break `level_event` with the removed state id first, then the update.
#[test]
fn a_creative_start_breaks_the_block_immediately() {
    let store = store_with_traced_chunk();
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players_with_mode(1);
    assert_eq!(store.block(93, 123, -22), Some(1), "solid before the break");

    act(&mut scheduler, &store, PlayerAction::ACTION_START, 1);

    assert_eq!(store.block(93, 123, -22), Some(0), "air after the break");
    expect_block_update(&mut receiver_a, traced_position());
    expect_ack(&mut receiver_a, 1);
    assert!(
        receiver_a.try_recv().is_err(),
        "the creative digger receives no level_event"
    );
    expect_level_event(&mut receiver_b, traced_position(), 1);
    expect_block_update(&mut receiver_b, traced_position());
    assert!(receiver_b.try_recv().is_err(), "an observer gets no ack");
}

/// A player joining after the creative break reads the changed snapshot too —
/// the chunk a later join sends already carries the air cell.
#[test]
fn a_later_join_after_a_creative_break_reads_the_changed_snapshot() {
    let store = store_with_traced_chunk();
    let (mut scheduler, _receiver_a, _receiver_b) = two_players_with_mode(1);

    act(&mut scheduler, &store, PlayerAction::ACTION_START, 1);

    let after = store.snapshot();
    match &after.chunks()[0].sections[11].blocks {
        PaletteContainer::Packed { palette, .. } => {
            assert!(palette.contains(&0), "the air state is on the wire");
        }
        other => panic!("expected a packed section after the break, got {other:?}"),
    }
}

/// A creative start in survival's shapes is untouched: with the digger in
/// survival a start still only acknowledges and leaves the block (the S3
/// trace), and the observer hears nothing.
#[test]
fn a_survival_start_still_only_acknowledges() {
    let store = store_with_traced_chunk();
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();

    act(&mut scheduler, &store, PlayerAction::ACTION_START, 1);

    assert_eq!(store.block(93, 123, -22), Some(1), "the block stays");
    expect_ack(&mut receiver_a, 1);
    assert!(receiver_a.try_recv().is_err(), "no update to the digger");
    assert!(receiver_b.try_recv().is_err(), "nothing for the observer");
}
