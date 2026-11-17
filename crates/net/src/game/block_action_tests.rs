//! Шаг 13b tests: a `player_action` (41) through the real per-tick path.
//!
//! The bytes themselves are checked in `protocol` against the vanilla S3
//! trace; here the subject is what the game thread does with the action —
//! which packets reach the digger and the observers, in which order, and
//! what a later join would read from the store.

use std::sync::Arc;

use data::{BlockHardness, BlockStates};
use protocol::{
    ChunkSection, HeightmapRecord, LevelChunkWithLight, LightData, PaletteContainer, PlayerAction,
    Position,
};
use tokio::sync::mpsc;

use crate::chunks::ReadyChunks;
use crate::store::ChunkStore;

use super::test_support::{join_at, join_at_mode};
use super::{
    block_action, movement::MoveUpdate, BlockAction, PlayerId, Region, Replay, Scheduler,
    ServerMessage, ToGame, SINGLE_REGION,
};

/// A block report with just air, stone and water, so the store can classify a
/// write and answer the steady-state water check.
fn test_blocks() -> Arc<BlockStates> {
    let text = r#"{
      "minecraft:air": {"states": [{"id": 0, "default": true}]},
      "minecraft:stone": {"states": [{"id": 1, "default": true}]},
      "minecraft:water": {"states": [{"id": 2, "default": true}]}
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
    // The timer's steady-state gate reads the last movement packet: leave the
    // digger standing on dry ground, exactly the traced dig condition.
    set_digger_pose(&mut scheduler, 0.5, 64.0, 0.5, MoveUpdate::ON_GROUND);
    (scheduler, receiver_a, receiver_b)
}

/// Puts player 1 at `(x, y, z)` with the given movement flags — the state the
/// steady-state gate in [`block_action::finish`] reads (`on_ground` bit and the
/// store's fluid surface at the feet).
fn set_digger_pose(scheduler: &mut Scheduler, x: f64, y: f64, z: f64, flags: u8) {
    let player = scheduler
        .region_mut(SINGLE_REGION)
        .and_then(|region| region.get_mut(1))
        .expect("the digger is in the region");
    player.apply_move(&MoveUpdate {
        x: Some(x),
        y: Some(y),
        z: Some(z),
        yaw: None,
        pitch: None,
        flags,
    });
}

/// The traced block of the S3 break.
fn traced_position() -> Position {
    Position {
        x: 93,
        y: 123,
        z: -22,
    }
}

/// Applies one action as player 1, with the timer off (no hardness report).
fn act(scheduler: &mut Scheduler, store: &Arc<ChunkStore>, action: i32, sequence: i32) {
    act_with(scheduler, store, None, None, action, sequence);
}

/// Applies one action as player 1 with explicit timer rules (Шаг 16a.3).
fn act_with(
    scheduler: &mut Scheduler,
    store: &Arc<ChunkStore>,
    hardness: Option<&BlockHardness>,
    tolerance_ticks: Option<u32>,
    action: i32,
    sequence: i32,
) {
    let mut replay = Replay::<Vec<u8>>::new(None);
    block_action::apply(
        scheduler,
        &mut replay,
        0,
        Some(store),
        super::dig::DigRules {
            hardness,
            tolerance_ticks,
        },
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
        super::dig::DigRules::trusting(),
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
        super::dig::DigRules::trusting(),
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
            block_action::apply(
                &mut scheduler,
                &mut replay,
                0,
                Some(&store),
                super::dig::DigRules::trusting(),
                action,
            );
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

// === Шаг 16a.3: the server-side mining timer ==============================

/// A hardness report for the test store: state 1 (stone) gets `hardness` and
/// a bare-hand speed of 1.0; air (0) is instant.
fn test_hardness(hardness: f64) -> Arc<BlockHardness> {
    let text = format!(
        r#"{{"states": 2, "hardness": {{"1": {hardness}, "0": 0.0}}, "handSpeed": {{"1": 1.0, "0": 1.0}}}}"#
    );
    Arc::new(BlockHardness::parse(&text, "test".to_owned()).expect("a valid report"))
}

/// The same report with state 1 flagged as requiring the right tool (stone in
/// the real report): a bare hand then digs at `/100` (Шаг 17a).
fn test_hardness_with_tool(hardness: f64) -> Arc<BlockHardness> {
    let text = format!(
        r#"{{"states": 2, "hardness": {{"1": {hardness}, "0": 0.0}}, "handSpeed": {{"1": 1.0, "0": 1.0}}, "toolRequired": [1]}}"#
    );
    Arc::new(BlockHardness::parse(&text, "test".to_owned()).expect("a valid report"))
}

/// Advances the mining timer one tick and returns the stage bytes the observer
/// (player 2) received this tick.
fn dig_tick(scheduler: &mut Scheduler, observer: &mut mpsc::Receiver<ServerMessage>) -> Vec<u8> {
    super::dig::tick(scheduler);
    let mut stages = Vec::new();
    while let Ok(message) = observer.try_recv() {
        match message {
            ServerMessage::BlockDestruction { stage, .. } => stages.push(stage),
            other => panic!("only crack stages reach the observer here, got {other:?}"),
        }
    }
    stages
}

/// Expects one `block_destruction` on the queue.
fn expect_stage(
    receiver: &mut mpsc::Receiver<ServerMessage>,
    digger: PlayerId,
    position: Position,
    stage: u8,
) {
    match receiver.try_recv().expect("a block_destruction") {
        ServerMessage::BlockDestruction {
            digger: got_digger,
            position: got_position,
            stage: got_stage,
        } => {
            assert_eq!(got_digger, digger, "the digging player's entity id");
            assert_eq!(got_position, position, "the block being dug");
            assert_eq!(got_stage, stage, "the crack stage");
        }
        other => panic!("expected block_destruction, got {other:?}"),
    }
}

/// Expects `block_update(position, state)` — the rollback of a refused break
/// carries the block's real state, not air.
fn expect_block_state(
    receiver: &mut mpsc::Receiver<ServerMessage>,
    position: Position,
    state: i32,
) {
    match receiver.try_recv().expect("a block_update") {
        ServerMessage::BlockUpdate {
            position: got,
            state: got_state,
        } => {
            assert_eq!(got, position, "the traced position");
            assert_eq!(got_state, state, "the block's real state");
        }
        other => panic!("expected block_update, got {other:?}"),
    }
}

/// The leaves dig of the trace (hardness 0.2, `T = 6` ticks): the observer
/// sees stages 1, 3, 5, 6, 8, 10, 11 — `floor(progress·10)` unclamped — while
/// the digger only gets its start ack. Progress passes 1.0 without the server
/// breaking the block; the client's finish does. [traced stages, §1.7]
#[test]
fn a_timed_leaves_dig_sends_the_traced_stage_sequence() {
    let store = store_with_traced_chunk();
    let hardness = test_hardness(0.2);
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();

    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_START,
        1,
    );
    expect_ack(&mut receiver_a, 1);
    assert!(
        receiver_b.try_recv().is_err(),
        "no stage before the first tick"
    );

    for stage in [1u8, 3, 5, 6, 8, 10, 11] {
        assert_eq!(
            dig_tick(&mut scheduler, &mut receiver_b),
            vec![stage],
            "the traced stage sequence"
        );
        assert!(
            receiver_a.try_recv().is_err(),
            "the digger never receives its own cracks"
        );
    }
    assert_eq!(
        store.block(93, 123, -22),
        Some(1),
        "the timer alone never breaks the block"
    );

    // Progress is 7/6: the finish is on time within the 3-tick tolerance. The
    // observer sees the reset before the break effect, as the trace does.
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_FINISH,
        2,
    );
    expect_block_update(&mut receiver_a, traced_position());
    expect_ack(&mut receiver_a, 2);
    expect_stage(&mut receiver_b, 1, traced_position(), 0xFF);
    expect_level_event(&mut receiver_b, traced_position(), 1);
    expect_block_update(&mut receiver_b, traced_position());
    assert_eq!(
        store.block(93, 123, -22),
        Some(0),
        "the finish removes the block"
    );
}

/// Dirt (0.5, `T = 15`) starts at stage 0: the first tick's progress is
/// 1/15, so `floor(progress·10)` is 0; the second tick crosses to 1.
#[test]
fn a_slow_block_starts_at_stage_zero() {
    let store = store_with_traced_chunk();
    let hardness = test_hardness(0.5);
    let (mut scheduler, _receiver_a, mut receiver_b) = two_players();
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_START,
        1,
    );

    assert_eq!(
        dig_tick(&mut scheduler, &mut receiver_b),
        vec![0],
        "dirt starts at 0"
    );
    assert_eq!(
        dig_tick(&mut scheduler, &mut receiver_b),
        vec![1],
        "two ticks of 1/15 cross the first tenth"
    );
}

/// A finish one tick into a 60-tick log dig is refused (tolerance 0): the
/// observer gets the crack reset, the digger the real state corrected and the
/// ack, and the block stays. [hypothesis] — no trace shows a refusal.
#[test]
fn an_early_finish_is_refused_and_rolled_back() {
    let store = store_with_traced_chunk();
    let hardness = test_hardness(2.0);
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(0),
        PlayerAction::ACTION_START,
        1,
    );
    expect_ack(&mut receiver_a, 1);
    assert_eq!(dig_tick(&mut scheduler, &mut receiver_b), vec![0]);

    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(0),
        PlayerAction::ACTION_FINISH,
        2,
    );

    expect_block_state(&mut receiver_a, traced_position(), 1);
    expect_ack(&mut receiver_a, 2);
    expect_stage(&mut receiver_b, 1, traced_position(), 0xFF);
    assert!(
        receiver_b.try_recv().is_err(),
        "a refused break never reaches the observer"
    );
    assert_eq!(store.block(93, 123, -22), Some(1), "the block stays");
}

/// The 3-tick tolerance lets a finish slightly before `1.0` through: 58 of
/// the log's 60 ticks are enough (the plan's soft option (a)). [hypothesis]
#[test]
fn the_finish_tolerance_accepts_a_slightly_early_dig() {
    let store = store_with_traced_chunk();
    let hardness = test_hardness(2.0);
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_START,
        1,
    );
    expect_ack(&mut receiver_a, 1);
    for _ in 0..58 {
        let _ = dig_tick(&mut scheduler, &mut receiver_b);
    }

    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_FINISH,
        2,
    );

    expect_block_update(&mut receiver_a, traced_position());
    expect_ack(&mut receiver_a, 2);
    assert_eq!(store.block(93, 123, -22), Some(0), "the tolerance accepts");
}

/// A stone dig with no tool (Шаг 17a): the report flags state 1, so the step
/// is `1/1.5/100` and the stages run 0..10 over **150** ticks, not 45. The
/// traced client takes 151 ticks (live `2026-11-17_0142_s12-ours`).
#[test]
fn a_tool_required_block_digs_at_the_hundred_divisor() {
    let store = store_with_traced_chunk();
    let hardness = test_hardness_with_tool(1.5);
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();

    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_START,
        1,
    );
    expect_ack(&mut receiver_a, 1);

    let mut first_seen = std::collections::BTreeMap::new();
    // The real client sends its finish on tick 151 (151 measured live); the
    // float accumulation also crosses stage 10 on 151, not exactly 150.
    for tick in 1..=151 {
        for stage in dig_tick(&mut scheduler, &mut receiver_b) {
            first_seen.entry(stage).or_insert(tick);
        }
    }
    assert_eq!(
        first_seen.keys().copied().collect::<Vec<_>>(),
        (0..=10).collect::<Vec<_>>(),
        "stages 0..10, one per fifteenth of the dig"
    );
    for (stage, tick) in &first_seen {
        let expected = 15 * i32::from(*stage);
        assert!(
            (*tick - expected).abs() <= 1,
            "stage {stage} first at tick {tick}, expected {expected} ± 1 (T = 100·hardness = 150, not 45)"
        );
    }
    assert_eq!(
        store.block(93, 123, -22),
        Some(1),
        "the timer alone never breaks the block"
    );

    // Progress is just past 1.0 at 151 ticks: the finish is on time (the real
    // client sent its finish on the same tick).
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_FINISH,
        2,
    );
    expect_block_update(&mut receiver_a, traced_position());
    expect_ack(&mut receiver_a, 2);
    expect_stage(&mut receiver_b, 1, traced_position(), 0xFF);
    assert_eq!(
        store.block(93, 123, -22),
        Some(0),
        "an honest stone dig is accepted"
    );
}

/// A stone finish at the old `/30` time (45 ticks) is refused: the timer is
/// only a third of the way. This is the bug Шаг 17a closes — the observer used
/// to see the full crack at ~44 ticks and wait for 106 more.
#[test]
fn a_stone_finish_at_the_thirty_divisor_time_is_refused() {
    let store = store_with_traced_chunk();
    let hardness = test_hardness_with_tool(1.5);
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_START,
        1,
    );
    expect_ack(&mut receiver_a, 1);
    for _ in 0..45 {
        let _ = dig_tick(&mut scheduler, &mut receiver_b);
    }

    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_FINISH,
        2,
    );

    expect_block_state(&mut receiver_a, traced_position(), 1);
    expect_ack(&mut receiver_a, 2);
    expect_stage(&mut receiver_b, 1, traced_position(), 0xFF);
    assert_eq!(
        store.block(93, 123, -22),
        Some(1),
        "45 ticks are not enough for a bare-hand stone dig"
    );
}

/// An abort stops the timer and clears the cracks with `0xFF`; the block stays
/// and no break packet goes out. [traced reset on an interrupted dig]
#[test]
fn an_abort_resets_the_cracks() {
    let store = store_with_traced_chunk();
    let hardness = test_hardness(0.2);
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_START,
        1,
    );
    expect_ack(&mut receiver_a, 1);
    assert_eq!(dig_tick(&mut scheduler, &mut receiver_b), vec![1]);

    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_ABORT,
        2,
    );

    expect_ack(&mut receiver_a, 2);
    expect_stage(&mut receiver_b, 1, traced_position(), 0xFF);
    assert!(
        receiver_b.try_recv().is_err(),
        "an abort never breaks the block"
    );
    assert_eq!(store.block(93, 123, -22), Some(1), "the block stays");

    // The timer is gone: a later finish is trusted as before (§4.2 trust).
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_FINISH,
        3,
    );
    expect_block_update(&mut receiver_a, traced_position());
    expect_ack(&mut receiver_a, 3);
}

/// A player who quits mid-dig stops showing cracks to the observer: the quit
/// takes the dig state with it and resets the cracks first. [hypothesis] — the
/// traces have no quit case.
#[test]
fn a_quit_mid_dig_resets_the_cracks() {
    let store = store_with_traced_chunk();
    let hardness = test_hardness(0.2);
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_START,
        1,
    );
    expect_ack(&mut receiver_a, 1);
    assert_eq!(dig_tick(&mut scheduler, &mut receiver_b), vec![1]);

    let mut replay = Replay::<Vec<u8>>::new(None);
    let config = super::GameConfig {
        store: Some(store.clone()),
        hardness: Some(hardness.clone()),
        ..super::GameConfig::default()
    };
    super::apply(
        &mut scheduler,
        &mut replay,
        0,
        ToGame::Quit { id: 1 },
        &config,
    );

    expect_stage(&mut receiver_b, 1, traced_position(), 0xFF);
    assert!(
        scheduler.regions()[0].get(1).is_none(),
        "the dig state goes away with the player"
    );
    assert!(
        receiver_a.try_recv().is_err(),
        "the digger's own queue ends with it"
    );
}

/// A state the report marks unbreakable (`hardness < 0`) is never timed and
/// never removed: the start only acknowledges, the finish corrects the digger
/// with the real state. [hypothesis] — no trace case.
#[test]
fn an_unbreakable_block_is_never_removed() {
    let store = store_with_traced_chunk();
    let hardness = test_hardness(-1.0);
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();

    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_START,
        1,
    );
    expect_ack(&mut receiver_a, 1);
    assert!(receiver_b.try_recv().is_err(), "no timer, no stages");

    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_FINISH,
        2,
    );
    expect_block_state(&mut receiver_a, traced_position(), 1);
    expect_ack(&mut receiver_a, 2);
    assert!(receiver_b.try_recv().is_err(), "the observer hears nothing");
    assert_eq!(store.block(93, 123, -22), Some(1), "bedrock stays bedrock");
}

/// `hardness = 0` is an instant block: no timer starts, no cracks are drawn,
/// and the client's finish removes it with the usual traced packets.
#[test]
fn an_instant_block_breaks_without_a_timer() {
    let store = store_with_traced_chunk();
    let hardness = test_hardness(0.0);
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();

    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_START,
        1,
    );
    expect_ack(&mut receiver_a, 1);
    assert!(
        receiver_b.try_recv().is_err(),
        "an instant block has no stages"
    );

    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(3),
        PlayerAction::ACTION_FINISH,
        2,
    );
    expect_block_update(&mut receiver_a, traced_position());
    expect_ack(&mut receiver_a, 2);
    expect_level_event(&mut receiver_b, traced_position(), 1);
    expect_block_update(&mut receiver_b, traced_position());
    assert_eq!(store.block(93, 123, -22), Some(0), "the block is gone");
}

// === Шаг 16a.4: the steady-state gate =====================================

/// [`store_with_traced_chunk`] plus a chunk `(0, 0)` whose section slot 8
/// (y 64...79) is water: the digger of [`two_players`] stands at y = 64 with
/// its feet inside it.
fn store_with_water_at_the_diggers_feet() -> Arc<ChunkStore> {
    let mut chunk = solid_chunk(0, 0);
    chunk.sections[8] = ChunkSection {
        non_air: 4096,
        fluid: 4096,
        blocks: PaletteContainer::Single(2),
        biomes: PaletteContainer::Single(1),
    };
    Arc::new(ChunkStore::new(
        ReadyChunks::new(vec![chunk, solid_chunk(5, -2)]),
        test_blocks(),
    ))
}

/// A finish while the digger is airborne is trusted even with `tolerance = 0`:
/// vanilla's client divides its own mining speed by five in the air, so the
/// bare-hand ground timer cannot judge that dig. The stages still follow the
/// ground cadence. [hypothesis: no airborne dig trace; plan §6 п.8 option (b),
/// recorded as debt in `docs/BLOCK-BREAKING.md` §4.3]
#[test]
fn an_airborne_finish_is_trusted_and_breaks_the_block() {
    let store = store_with_traced_chunk();
    let hardness = test_hardness(2.0);
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(0),
        PlayerAction::ACTION_START,
        1,
    );
    expect_ack(&mut receiver_a, 1);
    assert_eq!(dig_tick(&mut scheduler, &mut receiver_b), vec![0]);

    // The client jumped: its last movement packet says "not on the ground".
    set_digger_pose(&mut scheduler, 0.5, 65.0, 0.5, 0);
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(0),
        PlayerAction::ACTION_FINISH,
        2,
    );

    expect_block_update(&mut receiver_a, traced_position());
    expect_ack(&mut receiver_a, 2);
    expect_stage(&mut receiver_b, 1, traced_position(), 0xFF);
    expect_level_event(&mut receiver_b, traced_position(), 1);
    expect_block_update(&mut receiver_b, traced_position());
    assert_eq!(
        store.block(93, 123, -22),
        Some(0),
        "an airborne finish is not rolled back even at tolerance 0"
    );
}

/// A finish while the feet are in water is trusted the same way, and the
/// on-ground bit alone does not hide it: the digger stands on the ground
/// (`flags = ON_GROUND`) but the store says water at the feet, so the strict
/// check is skipped. [hypothesis: no water dig trace; plan §6 п.8 option (b)]
#[test]
fn a_finish_with_the_feet_in_water_is_trusted() {
    let store = store_with_water_at_the_diggers_feet();
    let hardness = test_hardness(2.0);
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(0),
        PlayerAction::ACTION_START,
        1,
    );
    expect_ack(&mut receiver_a, 1);
    assert_eq!(dig_tick(&mut scheduler, &mut receiver_b), vec![0]);

    // On the ground, but the feet cell is water (chunk (0, 0), section slot 8).
    set_digger_pose(&mut scheduler, 0.5, 64.0, 0.5, MoveUpdate::ON_GROUND);
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(0),
        PlayerAction::ACTION_FINISH,
        2,
    );

    expect_block_update(&mut receiver_a, traced_position());
    expect_ack(&mut receiver_a, 2);
    expect_stage(&mut receiver_b, 1, traced_position(), 0xFF);
    expect_level_event(&mut receiver_b, traced_position(), 1);
    expect_block_update(&mut receiver_b, traced_position());
    assert_eq!(
        store.block(93, 123, -22),
        Some(0),
        "a water feet finish is not rolled back even at tolerance 0"
    );
}

/// The gate's control case: the same pose without the water cell is steady, so
/// the identical early finish is still refused — water is the only difference
/// between the two tests. [hypothesis refusal, §4.2]
#[test]
fn the_same_pose_on_dry_ground_is_still_refused() {
    let store = store_with_traced_chunk();
    let hardness = test_hardness(2.0);
    let (mut scheduler, mut receiver_a, mut receiver_b) = two_players();
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(0),
        PlayerAction::ACTION_START,
        1,
    );
    expect_ack(&mut receiver_a, 1);
    assert_eq!(dig_tick(&mut scheduler, &mut receiver_b), vec![0]);

    set_digger_pose(&mut scheduler, 0.5, 64.0, 0.5, MoveUpdate::ON_GROUND);
    act_with(
        &mut scheduler,
        &store,
        Some(&hardness),
        Some(0),
        PlayerAction::ACTION_FINISH,
        2,
    );

    expect_block_state(&mut receiver_a, traced_position(), 1);
    expect_ack(&mut receiver_a, 2);
    expect_stage(&mut receiver_b, 1, traced_position(), 0xFF);
    assert!(
        receiver_b.try_recv().is_err(),
        "a refused break never reaches the observer"
    );
    assert_eq!(store.block(93, 123, -22), Some(1), "the block stays");
}
