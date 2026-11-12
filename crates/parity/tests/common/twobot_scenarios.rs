//! Scenario scripts of the M4.5 two-bot trace: what bot A observes and
//! what bot B does in С1/С2/С3 (`docs/ARCH-M4.md`, §6).
//!
//! С1 — B joins near A, stands 10 s, leaves by closing its connection
//! (the clean leave); A records what vanilla shows it.
//! С2 — B walks +x for 10 s (0.2 blocks, one packet per tick, tick
//! coupled), alternating `move_player_pos` / `move_player_pos_rot` /
//! `move_player_rot` with different yaw/pitch, then stands 5 s.
//! С3 — B walks +x until vanilla stops showing B to A (a
//! `remove_entities` of B's id while B did not leave: A arms that
//! detection only while B is walking), then walks back to the join point.
//!
//! Movement angles reuse the live client's observed shapes: the rot-only
//! payload layout comes from `live-session.json` (id 32, 9 bytes), the
//! yaw/pitch *values* are the script's own (different per packet type, so
//! head and body rotations are observable in A's trace).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use parity::TraceEntry;

use crate::twobot::{in_ready_zone, Behavior, Cmd, Shared};

/// Straight step per tick: the owner's per-tick cap (same as walk).
pub const STEP: f64 = 0.2;
/// Game mode id of spectator (both bots must prove it from their traces).
pub const EXPECTED_GAME_MODE: i32 = 3;
/// С3: ticks A is given after each join so its `add_entity` for B lands
/// before B starts walking (otherwise the loss could never be matched).
const WARM_TICKS: u64 = 40;
/// С3: ticks B stands at the join point after returning (5 s).
const HOME_STAND_TICKS: u64 = 100;

/// What a scenario asks bot B to do after the shared join phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scene {
    /// С1: stand 10 s, then leave cleanly.
    Join,
    /// С2: walk +x 10 s with rotation, stand 5 s.
    Walk,
    /// С3: walk until A loses track of B, then walk back.
    Track,
}

impl Scene {
    /// Short scenario code used in file names and reports.
    pub fn code(self) -> &'static str {
        match self {
            Scene::Join => "c1",
            Scene::Walk => "c2",
            Scene::Track => "c3",
        }
    }
}

/// Bot A's role: stay at spawn for the whole scenario and watch B.
///
/// Live detection is only armed for С3 through [`Shared::b_moving`]: a
/// `remove_entities` that carries B's entity id while B is walking flips
/// [`Shared::lost_contact`]. Entity identification prefers the hypothesised
/// layout (leading VarInt id, then the 16-byte offline UUID) and falls back
/// to "the first `add_entity` after B joined"; which one worked is recorded.
pub struct Observer {
    /// Coordination and observation state of the scenario.
    shared: Arc<Shared>,
    /// B's offline UUID bytes (= `protocol::offline_uuid(BOT_B)`).
    b_uuid: [u8; 16],
    /// Ticks counted after B's script signalled its end.
    after_script: u64,
}

impl Observer {
    /// An observer for one scenario, expecting bot B's named UUID.
    pub fn new(shared: Arc<Shared>, b_uuid: [u8; 16]) -> Self {
        Self {
            shared,
            b_uuid,
            after_script: 0,
        }
    }
}

impl Behavior for Observer {
    fn on_joined(&mut self) {
        self.shared.a_joined.store(true, Ordering::SeqCst);
    }

    fn on_teleport(&mut self, _pos: [f64; 3]) {}

    fn on_packet(&mut self, entry: &TraceEntry, payload: &[u8]) -> Vec<Cmd> {
        match entry.name.as_deref() {
            Some("minecraft:add_entity") => {
                let index = self.shared.adds.fetch_add(1, Ordering::SeqCst);
                if self.shared.b_entity.load(Ordering::SeqCst) >= 0 {
                    return Vec::new();
                }
                // Hypothesis first: entity id (VarInt) + 16-byte UUID.
                let hypothesised = match protocol::decode_varint(payload) {
                    Ok((id, at)) if payload.get(at..at + 16) == Some(self.b_uuid.as_slice()) => {
                        Some(id)
                    }
                    _ => None,
                };
                match hypothesised {
                    Some(id) => {
                        self.shared.uuid_strategy.store(true, Ordering::SeqCst);
                        self.shared
                            .entity_add_index
                            .store(index as i32, Ordering::SeqCst);
                        self.shared.b_entity.store(id, Ordering::SeqCst);
                    }
                    None if self.shared.b_joined.load(Ordering::SeqCst) => {
                        // Fallback: the first add_entity after B is in play.
                        if let Ok((id, _)) = protocol::decode_varint(payload) {
                            self.shared
                                .entity_add_index
                                .store(index as i32, Ordering::SeqCst);
                            self.shared.b_entity.store(id, Ordering::SeqCst);
                        }
                    }
                    None => {}
                }
            }
            Some("minecraft:remove_entities") => {
                self.shared.removes.fetch_add(1, Ordering::SeqCst);
                let b = self.shared.b_entity.load(Ordering::SeqCst);
                let armed = self.shared.b_moving.load(Ordering::SeqCst);
                let lost = self.shared.lost_contact.load(Ordering::SeqCst);
                if b < 0 || !armed || lost {
                    return Vec::new();
                }
                match crate::twobot::decode_ids(payload) {
                    Ok(ids) => {
                        if ids.contains(&b) {
                            self.shared.lost_contact.store(true, Ordering::SeqCst);
                        }
                    }
                    Err(error) => eprintln!("A: cannot parse remove_entities: {error}"),
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn on_tick(&mut self, loaded: bool, _pos: Option<[f64; 3]>) -> Vec<Cmd> {
        if !loaded || !self.shared.script_done.load(Ordering::SeqCst) {
            return Vec::new();
        }
        self.after_script += 1;
        if self.after_script >= 100 {
            // 5 s of observation after B's script ended (in С1 that
            // covers B's connection close and its server-side effects).
            return vec![Cmd::Stop(
                "A stayed at spawn and observed 5 s after B finished".to_owned(),
            )];
        }
        Vec::new()
    }
}

/// Bot B's script for one scenario: pure state; all I/O and timers live
/// in `crate::twobot::run_bot`.
pub struct BScript {
    /// Which script to run.
    scene: Scene,
    /// Coordination state of the scenario.
    shared: Arc<Shared>,
    /// Ticks after `player_loaded` (movement is gated on it).
    ticks: u64,
    /// The join position (walk target of the С3 return).
    origin: Option<[f64; 3]>,
    /// С3: set when B noticed the lost contact and started walking back.
    loss_at: Option<[f64; 3]>,
    /// С3: ticks spent standing at the origin after returning.
    home_ticks: u64,
}

impl BScript {
    /// The script of `scene` for one scenario.
    pub fn new(scene: Scene, shared: Arc<Shared>) -> Self {
        Self {
            scene,
            shared,
            ticks: 0,
            origin: None,
            loss_at: None,
            home_ticks: 0,
        }
    }

    /// С2: one tick of the walk/rotate cycle — advance on a plain or a
    /// pos+rot packet, a rotation in place every third tick (exactly one
    /// movement packet per tick). The ready-zone assert runs before the
    /// step is returned, i.e. before it can be sent.
    fn walk_rotate(&mut self, pos: [f64; 3]) -> Vec<Cmd> {
        let mut next = pos;
        next[0] += STEP;
        if !in_ready_zone(next[0], next[2]) {
            self.shared.script_done.store(true, Ordering::SeqCst);
            return vec![Cmd::Bounds(
                "bounds: the next С2 step would leave the ready zone".to_owned(),
            )];
        }
        match self.ticks % 3 {
            0 => vec![Cmd::MovePos(next)],
            1 => vec![Cmd::MovePosRot(next, 45.0, -20.0)],
            _ => vec![Cmd::MoveRot(-135.0, 30.0)],
        }
    }

    /// С3: forward walk arm (only armed after the warm ticks, only while
    /// losing contact has not been observed).
    fn track_forward(&mut self, pos: [f64; 3]) -> Vec<Cmd> {
        if self.ticks <= WARM_TICKS {
            // Let A register `add_entity` for B before B starts moving.
            return Vec::new();
        }
        self.shared.b_moving.store(true, Ordering::SeqCst);
        let mut next = pos;
        next[0] += STEP;
        if !in_ready_zone(next[0], next[2]) {
            self.shared.script_done.store(true, Ordering::SeqCst);
            return vec![Cmd::Bounds(
                "bounds: the next step would leave the ready zone before any tracking \
                 loss was observed — tracking distance: не установлено"
                    .to_owned(),
            )];
        }
        vec![Cmd::MovePos(next)]
    }

    /// С3: walk back toward the join point, then stand.
    fn track_home(&mut self, pos: [f64; 3]) -> Vec<Cmd> {
        self.shared.b_moving.store(false, Ordering::SeqCst);
        if self.loss_at.is_none() {
            self.loss_at = Some(pos);
        }
        let origin = self.origin.unwrap_or(pos);
        let dx = origin[0] - pos[0];
        let dz = origin[2] - pos[2];
        if dx.abs() < STEP / 2.0 && dz.abs() < STEP / 2.0 {
            self.home_ticks += 1;
            if self.home_ticks >= HOME_STAND_TICKS {
                self.shared.script_done.store(true, Ordering::SeqCst);
                let loss = self.loss_at.unwrap_or(pos);
                let distance = loss[0] - origin[0];
                return vec![Cmd::Stop(format!(
                    "B walked back to the join point; the tracking loss (remove_entities \
                     of B's id while B was walking) was observed {distance:.0} blocks along +x"
                ))];
            }
            return Vec::new();
        }
        let mut next = pos;
        if dx.abs() >= STEP / 2.0 {
            next[0] += dx.clamp(-STEP, STEP);
        } else {
            next[2] += dz.clamp(-STEP, STEP);
        }
        vec![Cmd::MovePos(next)]
    }
}

impl Behavior for BScript {
    fn on_joined(&mut self) {
        self.shared.b_joined.store(true, Ordering::SeqCst);
    }

    fn on_teleport(&mut self, _pos: [f64; 3]) {}

    fn on_packet(&mut self, _entry: &TraceEntry, _payload: &[u8]) -> Vec<Cmd> {
        Vec::new()
    }

    fn on_tick(&mut self, loaded: bool, pos: Option<[f64; 3]>) -> Vec<Cmd> {
        if !loaded {
            return Vec::new();
        }
        let Some(pos) = pos else {
            return Vec::new();
        };
        self.ticks += 1;
        match self.scene {
            Scene::Join => {
                if self.ticks >= 200 {
                    // 10 s standing after the load (walk's pace: 20 ticks/s).
                    self.shared.script_done.store(true, Ordering::SeqCst);
                    vec![Cmd::Stop(
                        "B stood 10 s after joining; its connection is closed next \
                         (the clean leave of С1)"
                            .to_owned(),
                    )]
                } else {
                    Vec::new()
                }
            }
            Scene::Walk => {
                if self.ticks <= 200 {
                    let cmds = self.walk_rotate(pos);
                    let moving = cmds
                        .iter()
                        .any(|cmd| matches!(cmd, Cmd::MovePos(..) | Cmd::MovePosRot(..)));
                    self.shared.b_moving.store(moving, Ordering::SeqCst);
                    cmds
                } else if self.ticks <= 300 {
                    self.shared.b_moving.store(false, Ordering::SeqCst);
                    Vec::new()
                } else {
                    self.shared.script_done.store(true, Ordering::SeqCst);
                    vec![Cmd::Stop(
                        "B walked +x for 10 s (0.2 blocks per tick, one packet per tick, \
                         pos / pos_rot / rot rotation cycle) and stood 5 s"
                            .to_owned(),
                    )]
                }
            }
            Scene::Track => {
                if self.origin.is_none() {
                    self.origin = Some(pos);
                }
                if self.shared.lost_contact.load(Ordering::SeqCst) {
                    return self.track_home(pos);
                }
                self.track_forward(pos)
            }
        }
    }
}
