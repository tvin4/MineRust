//! Scheduler-level integration coverage for accepted swings reaching observers.

use std::time::Instant;

use super::test_support::join_at;
use super::{Scheduler, ServerMessage, SINGLE_REGION};

#[test]
fn swings_reach_the_observer_through_the_scheduler() {
    let mut scheduler = Scheduler::new(vec![super::Region::new(SINGLE_REGION)]);
    let (join_a, slot_a, mut rx_a) = join_at(1, (0, 64, 0));
    let (join_b, _slot_b, mut rx_b) = join_at(2, (5, 64, 0));
    scheduler.insert_player(join_a);
    scheduler.insert_player(join_b);
    scheduler.confirm_teleport(1);
    scheduler.confirm_teleport(2);

    // Tick 0: the mutual spawn pair.
    let outcome = scheduler.tick(0, Instant::now());
    let spawns: Vec<_> = outcome
        .entities
        .iter()
        .flat_map(|e| e.spawn.iter().map(|s| (e.player, s.id)))
        .collect();
    assert_eq!(spawns, vec![(1, 2), (2, 1)], "both see each other");
    // Drain both queues.
    while rx_a.try_recv().is_ok() {}
    while rx_b.try_recv().is_ok() {}

    // Tick 1: A punches three times.
    slot_a.store_swing();
    slot_a.store_swing();
    slot_a.store_swing();
    let outcome = scheduler.tick(1, Instant::now());
    let swings: Vec<_> = outcome
        .entities
        .iter()
        .map(|e| (e.player, e.swings.clone()))
        .collect();
    assert_eq!(
        swings,
        vec![(2, vec![1, 1, 1])],
        "observer hears the swings"
    );

    // Deliver exactly like the game loop does (game::deliver_entities).
    for event in &outcome.entities {
        let message = ServerMessage::StreamEntities {
            spawn: event.spawn.clone(),
            states: event.states.clone(),
            remove: event.remove.clone(),
            moves: event.moves.clone(),
            swings: event.swings.clone(),
        };
        scheduler.deliver(event.player, message);
    }
    let mut got = 0;
    while let Ok(message) = rx_b.try_recv() {
        if let ServerMessage::StreamEntities { swings, .. } = message {
            got += swings.len();
        }
    }
    assert_eq!(got, 3, "three swing packets queued for B");
}
