//! The stage-interval oracle and the tolerance derivation (Шаг 16a.4 п.3a/п.4).
//!
//! The ten vanilla digs of 2026-11-14 are re-read here as `(stage, tick offset)`
//! pairs and checked against the timer the server implements
//! (`super::dig::stage` advanced once per tick):
//!
//! - stage `k ≥ 1` goes out at tick `ceil(k·T/10)`, stage 0 at tick 1, where
//!   `T = 30·hardness` (the bare-hand formula of 16a.2);
//! - the proxy's `read_ms` carries one constant delivery offset per dig (the
//!   observer's clock starts with its own join), so every dig fits its own
//!   offset — the median of `model − observed` over its stages;
//! - after that offset every stage must land within **±1 tick** of the model,
//!   every *transition* within ±1 tick of the model interval, and the
//!   first-to-last span within ±1 tick.
//!
//! The same probes derive the finish tolerance: `start→finish − T` ranges
//! −1.93...+0.41 ticks over the ten digs, so `Some(3)` accepts all of them while
//! `Some(0)` would roll back five. The traces never show a refusal — the
//! tolerance stays a **hypothesis** (plan §6 п.8).
//!
//! SOURCE: `E:\MineRust-data\autotests\2026-11-14_{1303,1451,1606,1636,1644}\
//! mcproxy\proxy-conn-*.jsonl`, the observer connection (the only one that
//! receives `block_destruction`). Tick = 50 ms.

/// One dig as the proxy measured it, all times in ticks.
struct Dig {
    /// Which run and block, for failure messages.
    label: &'static str,
    /// `30 · hardness / handSpeed` — the modelled dig length.
    expected: f64,
    /// `(stage, tick offset from the dig's first stage packet)`.
    stages: &'static [(u8, f64)],
    /// The client's `start → finish` as the proxy read it.
    finish: f64,
}

const LEAVES: f64 = 6.0;
const DIRT: f64 = 15.0;
const GRASS: f64 = 18.0;

const DIGS: [Dig; 10] = [
    Dig {
        label: "1303 dark_oak_leaves (97,144,-34)",
        expected: LEAVES,
        stages: &[
            (1, 0.00),
            (3, 0.37),
            (5, 1.37),
            (6, 2.35),
            (8, 3.27),
            (10, 4.33),
        ],
        finish: 5.00,
    },
    Dig {
        label: "1303 dark_oak_leaves (97,144,-33)",
        expected: LEAVES,
        stages: &[
            (1, 0.00),
            (3, 0.64),
            (5, 1.45),
            (6, 2.61),
            (8, 3.39),
            (10, 4.39),
            (11, 5.37),
        ],
        finish: 6.01,
    },
    Dig {
        label: "1303 dark_oak_leaves (98,144,-33)",
        expected: LEAVES,
        stages: &[
            (1, 0.00),
            (3, 0.37),
            (5, 1.36),
            (6, 2.38),
            (8, 3.39),
            (10, 4.32),
            (11, 5.35),
        ],
        finish: 6.01,
    },
    Dig {
        label: "1451 oak_leaves (98,138,-22)",
        expected: LEAVES,
        stages: &[
            (1, 0.00),
            (3, 0.32),
            (5, 1.05),
            (6, 2.15),
            (8, 3.07),
            (10, 4.15),
        ],
        finish: 5.24,
    },
    Dig {
        label: "1451 oak_leaves (98,137,-22)",
        expected: LEAVES,
        stages: &[
            (1, 0.00),
            (3, 0.33),
            (5, 1.33),
            (6, 2.23),
            (8, 3.28),
            (10, 4.26),
            (11, 5.26),
        ],
        finish: 6.01,
    },
    Dig {
        label: "1606 oak_leaves (97,138,-22)",
        expected: LEAVES,
        stages: &[
            (1, 0.00),
            (3, 0.42),
            (5, 1.36),
            (6, 2.34),
            (8, 3.31),
            (10, 4.40),
            (11, 5.98),
            (13, 6.68),
        ],
        finish: 6.41,
    },
    Dig {
        label: "1606 oak_leaves (97,137,-22)",
        expected: LEAVES,
        stages: &[
            (1, 0.00),
            (3, 0.30),
            (5, 1.43),
            (6, 2.31),
            (8, 3.58),
            (10, 4.28),
            (11, 5.32),
        ],
        finish: 5.62,
    },
    Dig {
        label: "1636 grass_block (95,127,-22)",
        expected: GRASS,
        stages: &[
            (0, 0.00),
            (1, 0.35),
            (2, 2.46),
            (3, 4.19),
            (4, 6.53),
            (5, 7.29),
            (6, 9.71),
            (7, 11.41),
            (8, 13.31),
            (9, 15.36),
        ],
        finish: 16.38,
    },
    Dig {
        label: "1636 dirt (95,126,-22)",
        expected: DIRT,
        stages: &[
            (0, 0.00),
            (1, 0.25),
            (2, 1.64),
            (3, 3.41),
            (4, 4.61),
            (5, 6.47),
            (6, 7.43),
            (7, 9.47),
            (8, 10.71),
            (9, 12.73),
            (10, 13.53),
        ],
        finish: 15.10,
    },
    Dig {
        label: "1644 grass_block (93,123,-22)",
        expected: GRASS,
        stages: &[
            (0, 0.00),
            (1, 0.40),
            (2, 4.72),
            (3, 5.17),
            (4, 6.30),
            (5, 7.38),
            (6, 9.35),
            (7, 11.29),
            (8, 13.49),
            (9, 15.33),
        ],
        finish: 16.07,
    },
];

/// The tick at which the model sends a stage byte.
fn model_tick(expected: f64, stage: u8) -> f64 {
    if stage == 0 {
        1.0
    } else {
        (f64::from(stage) * expected / 10.0).ceil()
    }
}

/// Median of `values` (average of the two middle ones for an even count).
fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).expect("finite values"));
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

/// The dig's constant delivery offset: median of `model − observed`.
fn delivery_offset(dig: &Dig) -> f64 {
    let mut residuals: Vec<f64> = dig
        .stages
        .iter()
        .map(|(stage, observed)| model_tick(dig.expected, *stage) - observed)
        .collect();
    median(&mut residuals)
}

/// The stage samples whose residual after the offset is worst, as
/// `(stage, deviation)`.
fn worst_stage(dig: &Dig) -> (u8, f64) {
    let offset = delivery_offset(dig);
    dig.stages
        .iter()
        .map(|(stage, observed)| {
            let residual = model_tick(dig.expected, *stage) - observed - offset;
            (*stage, residual.abs())
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).expect("finite values"))
        .expect("a dig has stages")
}

/// The oracle after the per-dig offset: every stage within ±1 tick of the
/// model and every transition within ±1 tick of the model interval. The 1644
/// grass dig is excluded here — its one delivery stall is its own test below.
#[test]
fn every_traced_dig_matches_the_stage_model_within_one_tick() {
    for dig in DIGS.iter().filter(|d| !d.label.starts_with("1644")) {
        let (stage, deviation) = worst_stage(dig);
        assert!(
            deviation <= 1.0,
            "{}: stage {stage} is {deviation:.2} ticks off the model",
            dig.label
        );
    }
}

/// Each run: the gap between two stages after removing the offset must match
/// the model's gap within ±1 tick — the transitions, not just the samples.
#[test]
fn every_traced_stage_transition_matches_the_model_interval() {
    for dig in DIGS.iter().filter(|d| !d.label.starts_with("1644")) {
        let offset = delivery_offset(dig);
        for pair in dig.stages.windows(2) {
            let (stage_a, observed_a) = pair[0];
            let (stage_b, observed_b) = pair[1];
            let observed = observed_b - observed_a;
            let modelled = model_tick(dig.expected, stage_b) - model_tick(dig.expected, stage_a);
            assert!(
                (observed - modelled).abs() <= 1.0,
                "{}: {}→{} took {observed:.2} ticks, model {modelled:.2} (offset {offset:.2})",
                dig.label,
                stage_a,
                stage_b
            );
        }
    }
}

/// The first-to-last span of each dig is within ±1 tick of its model span.
#[test]
fn every_traced_stage_span_matches_the_model() {
    for dig in DIGS.iter().filter(|d| !d.label.starts_with("1644")) {
        let (first_stage, first_observed) = dig.stages[0];
        let (last_stage, last_observed) = dig.stages[dig.stages.len() - 1];
        let observed = last_observed - first_observed;
        let modelled = model_tick(dig.expected, last_stage) - model_tick(dig.expected, first_stage);
        assert!(
            (observed - modelled).abs() <= 1.0,
            "{}: span {observed:.2} ticks, model {modelled:.2}",
            dig.label
        );
    }
}

/// The 1644 grass dig carries one **delivery stall**: a 4.32-tick gap between
/// stage 1 and stage 2 instead of the modelled 2. The oracle must flag it, not
/// quietly allow it: its worst deviation is > 2 ticks, while the other nine
/// stage samples of the same dig stay within ±1 tick of the same offset.
#[test]
fn the_1644_grass_dig_flags_its_delivery_stall() {
    let dig = DIGS
        .iter()
        .find(|d| d.label.starts_with("1644"))
        .expect("the stalled dig is in the table");
    let (stage, deviation) = worst_stage(dig);
    assert_eq!(stage, 2, "the stall is the stage-1 → stage-2 gap");
    assert!(
        deviation > 2.0,
        "the stall must be visible to the oracle, got {deviation:.2} ticks"
    );

    // Everything else in that dig fits the same offset within ±1 tick.
    let offset = delivery_offset(dig);
    for (stage, observed) in dig.stages.iter().filter(|(stage, _)| *stage != 2) {
        let residual = model_tick(dig.expected, *stage) - observed - offset;
        assert!(
            residual.abs() <= 1.0,
            "stage {stage} is {residual:.2} ticks off after excluding the stall"
        );
    }
}

/// The tolerance derivation (plan §6 п.8): `start→finish − T` over the ten
/// digs is −1.93...+0.41 ticks. `Some(3)` accepts every one, `Some(0)` refuses
/// five (three leaves at 5.00/5.24/5.62 of 6 and both grass at 16.38/16.07 of
/// 18). The data alone needs 2; 3 is the margin for the proxy's ±0.5–1-tick
/// read jitter and the f32 progress rounding.
#[test]
fn the_three_tick_tolerance_accepts_every_traced_finish() {
    let mut early = 0.0f64;
    let mut late = 0.0f64;
    let mut rejected_at_zero = 0;
    for dig in &DIGS {
        let deviation = dig.finish - dig.expected;
        early = early.min(deviation);
        late = late.max(deviation);
        assert!(
            deviation >= -3.0,
            "{}: a {deviation:.2}-tick early finish must pass tolerance 3",
            dig.label
        );
        if deviation < 0.0 {
            rejected_at_zero += 1;
        }
    }
    assert!(
        (-2.0..-1.5).contains(&early),
        "the measured worst-early finish is {early:.2} ticks"
    );
    assert!(
        (0.0..1.0).contains(&late),
        "the measured latest finish is {late:.2} ticks"
    );
    assert_eq!(
        rejected_at_zero, 5,
        "tolerance 0 would roll back five of the ten traced digs"
    );
}
