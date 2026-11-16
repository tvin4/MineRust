//! Шаг 14e/15 oracle: our swimming rule scored against the recorded vanilla run.
//!
//! The fixture in [`super::s10_fixture`] is one vanilla run's ground truth, in
//! two forms: [`S10_EVENTS`] is one row per `set_entity_data` flags byte Bob
//! received (Alice's input bits and the fluid at her feet at that instant), and
//! [`S10_TICKS`] is the same run as a 50 ms grid of Alice's state and the
//! flags Bob had last received. Both are scored here with a 50 ms (one tick)
//! tolerance.

use super::entities::{
    is_swimming, pose_and_flags, pose_of, state_flags, Forced, CRAWLING_HEIGHT, CROUCHING_HEIGHT,
    STANDING_HEIGHT,
};
use super::s10_fixture::{S10_EVENTS, S10_GEOMETRY, S10_TICKS};
use super::s11_fixture::S11_SAMPLES;

/// Input bits of the RLE tick fixture (`T_NO_OBS` is the no-observation mark).
const T_SPRINT: u8 = 0x01;
const T_SNEAK: u8 = 0x02;
const T_FEET: u8 = 0x04;
const T_POST: u8 = 0x08;
const T_NO_OBS: u16 = 0x100;

/// One expanded 50 ms sample of the RLE tick fixture.
struct Tick {
    geometry_index: u8,
    sprint: bool,
    sneak: bool,
    feet_in_water: bool,
    post_teleport: bool,
    observed: Option<u8>,
}

fn expand_ticks() -> Vec<Tick> {
    let mut out = Vec::new();
    for &(repeats, geometry_index, bits, observed) in S10_TICKS {
        let tick = Tick {
            geometry_index,
            sprint: bits & T_SPRINT != 0,
            sneak: bits & T_SNEAK != 0,
            feet_in_water: bits & T_FEET != 0,
            post_teleport: bits & T_POST != 0,
            observed: if observed == T_NO_OBS {
                None
            } else {
                Some(observed as u8)
            },
        };
        for _ in 0..repeats {
            out.push(Tick {
                geometry_index: tick.geometry_index,
                sprint: tick.sprint,
                sneak: tick.sneak,
                feet_in_water: tick.feet_in_water,
                post_teleport: tick.post_teleport,
                observed: tick.observed,
            });
        }
    }
    out
}

/// The recorded sequence, scored with a one-event tolerance.
///
/// A mismatch is accepted when our prediction equals the previous or the next
/// observed flags of the same geometry: the metadata is broadcast one event
/// late on both swim edges (the swim bit is set one event after the sprint bit
/// on entry, and cleared one event after it on exit), which `docs/SWIMMING.md`
/// §4 documents. With the Шаг 15 sprint model (the `player_command` stream,
/// not the `player_input` bit) no transition is further than one event out.
#[test]
fn our_swim_rule_matches_the_vanilla_trace_within_one_event() {
    let mut total = 0usize;
    let mut exact = 0usize;
    let mut within = 0usize;
    let mut beyond: Vec<(&str, u8, u8)> = Vec::new();
    let mut pre = 0usize;
    let mut post = 0usize;

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
            if event.post_teleport {
                post += 1;
            } else {
                pre += 1;
            }
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
         pre {pre} post {post}, beyond {beyond:?}",
        100.0 * exact as f64 / total as f64,
        100.0 * within as f64 / total as f64,
    );

    assert_eq!(total, 68, "the r3 trace recorded 68 flags transitions");
    assert_eq!((pre, post), (8, 60), "pre/post teleport split drifted");
    assert_eq!(within, 68, "within +-1 event, beyond: {beyond:?}");
    assert_eq!(exact, 52, "exact agreement drifted");
    assert!(beyond.is_empty(), "no event is further than one event out");
}

/// The 50 ms grid, scored exactly and within one tick, split pre/post teleport.
///
/// This is the stricter base the owner asked for in Шаг 15 п.1: the transition
/// metric above only samples at the instants vanilla decided to broadcast,
/// while this one also checks the quiet ticks in between.
#[test]
fn our_swim_rule_matches_the_vanilla_trace_within_one_tick() {
    let ticks = expand_ticks();
    let mut counts = [(0usize, 0usize, 0usize); 3]; // (exact, within, total)
    let mut beyond: Vec<(u8, u16, u8, u8)> = Vec::new();

    for (i, tick) in ticks.iter().enumerate() {
        let Some(observed) = tick.observed else {
            continue;
        };
        let predicted = state_flags(
            tick.sneak,
            tick.sprint,
            is_swimming(tick.feet_in_water, tick.sprint),
        );
        let part = if tick.post_teleport { 2 } else { 1 };
        counts[0].2 += 1; // all
        counts[part].2 += 1;
        if predicted == observed {
            counts[0].0 += 1;
            counts[0].1 += 1;
            counts[part].0 += 1;
            counts[part].1 += 1;
            continue;
        }
        let neighbours = [i.checked_sub(1), Some(i + 1)]
            .into_iter()
            .flatten()
            .filter_map(|j| ticks.get(j).and_then(|t| t.observed));
        if neighbours.clone().any(|value| value == predicted) {
            counts[0].1 += 1;
            counts[part].1 += 1;
        } else {
            beyond.push((
                tick.geometry_index,
                observed as u16,
                predicted,
                tick.sprint as u8,
            ));
        }
    }

    for (part, name) in [(0usize, "all"), (1, "pre"), (2, "post")] {
        let (exact, within, total) = counts[part];
        println!(
            "S10 ticks {name:<4}: exact {exact}/{total} ({:.1}%), within +-1 {within}/{total} \
             ({:.1}%)",
            100.0 * exact as f64 / total as f64,
            100.0 * within as f64 / total as f64,
        );
    }
    println!("S10 ticks beyond: {beyond:?}");

    assert_eq!(counts[0].2, 4633, "the r3 grid is 4633 ticks");
    assert_eq!(counts[0].0, 4605, "exact tick agreement drifted");
    assert_eq!(counts[0].1, 4629, "within +-1 tick drifted");
    // The four residual ticks are the 50-100 ms broadcast lag at a swim or
    // sneak edge, one of them a 52.9 ms sneak release that just misses the
    // one-tick window.
    assert_eq!(beyond.len(), 4, "beyond one tick: {beyond:?}");
    assert!(
        S10_GEOMETRY.len() >= 17,
        "the geometry index table is used by the RLE ticks",
    );
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

/// The headroom rule's thresholds, built from the measured hitbox heights.
#[test]
fn the_headroom_rule_uses_the_measured_heights() {
    // The measured heights are what the thresholds are made of: crawl below
    // the crouching hitbox, crouch below the standing one, standing above.
    assert_eq!(
        Forced::from_headroom(None),
        Forced::None,
        "no report, no rule"
    );
    assert_eq!(
        Forced::from_headroom(Some(1.0)),
        Forced::Crawl,
        "stone at the head cell"
    );
    assert_eq!(
        Forced::from_headroom(Some(1.5)),
        Forced::Crouch,
        "top slab at the head cell"
    );
    assert_eq!(
        Forced::from_headroom(Some(1.8)),
        Forced::None,
        "exactly standing fits"
    );
    assert_eq!(Forced::from_headroom(Some(2.0)), Forced::None, "open");
    // The thresholds are the observed heights, not round numbers: a block
    // one thousandth below the crouching hitbox still forces a crawl.
    assert_eq!(
        Forced::from_headroom(Some(CROUCHING_HEIGHT - 0.001)),
        Forced::Crawl
    );
    assert_eq!(
        Forced::from_headroom(Some(STANDING_HEIGHT - 0.001)),
        Forced::Crouch
    );
    const { assert!(CRAWLING_HEIGHT < CROUCHING_HEIGHT && CROUCHING_HEIGHT < STANDING_HEIGHT) };
    assert_eq!(
        (CRAWLING_HEIGHT, CROUCHING_HEIGHT, STANDING_HEIGHT),
        (0.6, 1.5, 1.8)
    );
}

/// The pose/flags composition of the forced rule, including the swim flag that
/// a forced crawl deliberately does not set.
#[test]
fn the_forced_pose_composes_with_sneak_and_water() {
    // A forced crawl is pose 3 but, on dry land, carries no swim flag.
    assert_eq!(
        pose_and_flags(false, false, false, Forced::Crawl),
        (3, 0x00)
    );
    // A wet crawl is the same pose with the swim flag.
    assert_eq!(pose_and_flags(false, false, true, Forced::Crawl), (3, 0x10));
    // A forced crouch is pose 5 with the crouch flag.
    assert_eq!(
        pose_and_flags(false, false, false, Forced::Crouch),
        (5, 0x02)
    );
    // The sneak key and a forced crouch agree.
    assert_eq!(
        pose_and_flags(true, false, false, Forced::Crouch),
        (5, 0x02)
    );
    // Standing with no input is pose 0 and no flags; sprint still rides along.
    assert_eq!(pose_and_flags(false, false, false, Forced::None), (0, 0x00));
    assert_eq!(pose_and_flags(false, true, false, Forced::None), (0, 0x08));
    // Falling in water without sprinting swims under the 14e rule, not the
    // forced one: the composition keeps both consistent.
    assert_eq!(pose_and_flags(false, false, true, Forced::None), (3, 0x10));
}

/// Шаг 15 п.5 oracle: the forced-pose rule scored against the recorded vanilla
/// S11 run.
///
/// Every recorded sample contributes `count` rows; the rule turns the recorded
/// head-block bottom and the held sneak key into a pose and flags, which must
/// equal what the vanilla client reported byte for byte. All 112 samples must
/// agree — there is no one-event tolerance here because the pose is forced by
/// the geometry at the player's current column, not by an input edge.
#[test]
fn the_forced_pose_rule_matches_the_vanilla_s11_run() {
    let mut total = 0u32;
    let mut mismatches: Vec<(&str, &str, u8, u8, u8, u8)> = Vec::new();
    for sample in S11_SAMPLES {
        let headroom = sample.head_bottom.map(|bottom| bottom + 1.0);
        let forced = Forced::from_headroom(headroom);
        let (pose, flags) = pose_and_flags(sample.sneak, false, false, forced);
        total += sample.count;
        if pose != sample.pose || flags != sample.flags {
            mismatches.push((
                sample.case,
                sample.phase,
                pose,
                sample.pose,
                flags,
                sample.flags,
            ));
        }
    }

    println!(
        "S11 oracle: {total}/{total} samples agree across {} cases; mismatches: {mismatches:?}",
        S11_SAMPLES.len()
    );

    assert_eq!(total, 112, "the r2 run recorded 112 samples");
    assert!(
        mismatches.is_empty(),
        "the forced-pose rule disagrees with vanilla: {mismatches:?}"
    );
}
