//! Recorded vanilla S11 run, run-length encoded, for the forced-pose oracle.
//!
//! Generated from the r2 S11 dataset
//! (`E:\MineRust-data\autotests\s11-vanilla-r2\s11-samples.json`), NOT
//! hand-written: [`S11_SAMPLES`] collapses the 112 recorded 250 ms samples of
//! Alice (one row per exact, contiguous observation) into the distinct cases.
//! Every row carries the input the rule under test reads — the bottom of the
//! block above the feet, and whether sneak was held — together with the pose
//! and the entity-flags bits the vanilla client itself reported. The rule is
//! [`super::entities::Forced::from_headroom`] plus
//! [`super::entities::pose_and_flags`].

/// One distinct observation of the S11 run (see the module docs).
pub struct S11Sample {
    /// Case label of the scenario (`S11-stone-1`, `S11-slab-15`).
    pub case: &'static str,
    /// Scenario phase the samples belong to.
    pub phase: &'static str,
    /// Number of contiguous 250 ms samples this row collapses.
    pub count: u32,
    /// Bottom of the collision shape of the block above the feet, relative to
    /// its cell; `None` means no block obstructs the head (air). The headroom
    /// the rule reads is `head_bottom + 1.0` over a feet cell full of air, or
    /// the cap (`>= 1.8`) when `None`.
    pub head_bottom: Option<f64>,
    /// Whether the sneak key was held during the samples.
    pub sneak: bool,
    /// Pose the vanilla client reported (0 standing, 3 swimming, 5 crouching).
    pub pose: u8,
    /// Crouch/swim entity-flags bits the vanilla client reported (`0x02`
    /// crouch, `0x10` swim; sprint is never set in S11).
    pub flags: u8,
}

/// The recorded sequence, in time order (r2, 112 samples over both cases).
//
// SOURCE: E:\MineRust-data\autotests\s11-vanilla-r2\s11-samples.json.
pub const S11_SAMPLES: &[S11Sample] = &[
    S11Sample {
        case: "S11-stone-1",
        phase: "open",
        count: 4,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-stone-1",
        phase: "block-above",
        count: 6,
        head_bottom: Some(0.0),
        sneak: false,
        pose: 3,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-stone-1",
        phase: "open-again",
        count: 3,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-stone-1",
        phase: "walk-in",
        count: 8,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-stone-1",
        phase: "under-roof",
        count: 3,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-stone-1",
        phase: "sneak",
        count: 4,
        head_bottom: None,
        sneak: true,
        pose: 5,
        flags: 0x02,
    },
    S11Sample {
        case: "S11-stone-1",
        phase: "sneak-off",
        count: 3,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-stone-1",
        phase: "walk-out",
        count: 10,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-stone-1",
        phase: "walk-out-stop",
        count: 2,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-stone-1",
        phase: "exit",
        count: 4,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-stone-1",
        phase: "block-above-dynamic",
        count: 6,
        head_bottom: Some(0.0),
        sneak: false,
        pose: 3,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-stone-1",
        phase: "block-above-dynamic-cleared",
        count: 3,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-slab-15",
        phase: "open",
        count: 4,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-slab-15",
        phase: "block-above",
        count: 6,
        head_bottom: Some(0.5),
        sneak: false,
        pose: 5,
        flags: 0x02,
    },
    S11Sample {
        case: "S11-slab-15",
        phase: "open-again",
        count: 3,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-slab-15",
        phase: "walk-in",
        count: 8,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-slab-15",
        phase: "under-roof",
        count: 3,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-slab-15",
        phase: "sneak",
        count: 4,
        head_bottom: None,
        sneak: true,
        pose: 5,
        flags: 0x02,
    },
    S11Sample {
        case: "S11-slab-15",
        phase: "sneak-off",
        count: 3,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-slab-15",
        phase: "walk-out",
        count: 10,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-slab-15",
        phase: "walk-out-stop",
        count: 2,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-slab-15",
        phase: "exit",
        count: 4,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
    S11Sample {
        case: "S11-slab-15",
        phase: "block-above-dynamic",
        count: 6,
        head_bottom: Some(0.5),
        sneak: false,
        pose: 5,
        flags: 0x02,
    },
    S11Sample {
        case: "S11-slab-15",
        phase: "block-above-dynamic-cleared",
        count: 3,
        head_bottom: None,
        sneak: false,
        pose: 0,
        flags: 0x00,
    },
];
