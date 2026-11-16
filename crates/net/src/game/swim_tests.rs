//! Шаг 14e oracle: our swimming rule scored against the recorded vanilla run.
//!
//! The events in [`super::s10_fixture`] are one vanilla run's ground truth —
//! for every `set_entity_data` flags byte Bob received, Alice's input bits and
//! the fluid at her feet at the same aligned instant. This test runs each
//! event through our rule (`is_swimming` + `state_flags`) and compares.

use super::entities::{is_swimming, pose_of, state_flags};
use super::s10_fixture::S10_EVENTS;

/// The recorded sequence, scored with a one-event tolerance.
///
/// A mismatch is accepted when our prediction equals the previous or the next
/// observed flags of the same geometry: the metadata is broadcast one event
/// late on both swim edges (the client sets the bit a tick after the server,
/// and clears it a tick late), which `docs/SWIMMING.md` §4 documents.
#[test]
fn our_swim_rule_matches_the_vanilla_trace_within_one_event() {
    let mut total = 0usize;
    let mut exact = 0usize;
    let mut within = 0usize;
    let mut beyond: Vec<(&str, u8, u8)> = Vec::new();

    // Entries that share a geometry are one contiguous run: the neighbours of
    // a mismatch are read within the run only.
    let mut start = 0usize;
    while start < S10_EVENTS.len() {
        let geometry = S10_EVENTS[start].geometry;
        let mut end = start;
        while end < S10_EVENTS.len() && S10_EVENTS[end].geometry == geometry {
            end += 1;
        }
        let run = &S10_EVENTS[start..end];
        for (i, event) in run.iter().enumerate() {
            total += 1;
            let predicted = state_flags(
                event.sneak,
                event.sprint,
                is_swimming(event.feet_in_water, event.sprint),
            );
            if predicted == event.flags {
                exact += 1;
                within += 1;
                continue;
            }
            let before = i.checked_sub(1).map(|j| run[j].flags);
            let after = run.get(i + 1).map(|next| next.flags);
            if before == Some(predicted) || after == Some(predicted) {
                within += 1;
            } else {
                beyond.push((event.geometry, predicted, event.flags));
            }
        }
        start = end;
    }

    println!(
        "S10 oracle: exact {exact}/{total} ({:.1}%), within +-1 {within}/{total} ({:.1}%), \
         beyond {beyond:?}",
        100.0 * exact as f64 / total as f64,
        100.0 * within as f64 / total as f64,
    );

    assert_eq!(total, 68, "the r3 trace recorded 68 flags transitions");
    assert_eq!(within, 66, "within +-1 event, beyond: {beyond:?}");
    assert_eq!(exact, 50, "exact agreement drifted");
    // The residual two: the client still reported the sprint input bit at a
    // geometry teleport while the server had already cleared the flag, so our
    // sprint bit (and therefore the 0x08 flag) is set where vanilla's is not.
    let residual: Vec<&str> = beyond.iter().map(|(geometry, _, _)| *geometry).collect();
    assert_eq!(residual, ["G5-bubble-column", "G5-seagrass"]);
}

/// The rule's truth table: only sprinting with the feet in water swims.
#[test]
fn only_sprinting_with_wet_feet_swims() {
    assert!(!is_swimming(false, false), "dry at rest");
    assert!(!is_swimming(false, true), "sprinting on dry land");
    assert!(!is_swimming(true, false), "standing in water");
    assert!(is_swimming(true, true), "sprinting in water");
}

/// The pose is the swim pose when swimming, else the sneak pose, else the
/// standing pose; swimming wins if both bits are somehow set.
#[test]
fn the_pose_prefers_swimming_over_sneaking() {
    assert_eq!(pose_of(false, false), 0, "standing");
    assert_eq!(pose_of(true, false), 5, "crouching");
    assert_eq!(pose_of(false, true), 3, "swimming");
    assert_eq!(pose_of(true, true), 3, "swimming wins over crouching");
}
