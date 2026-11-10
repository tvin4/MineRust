# HANDOFF (state for the next session)

M4.3 part A in progress (owner allowed part A only; part B waits for "M4.3 B можно").
- A1.1 done (code reading): the drift 261→1.8 was sequential catch-up, not a reset — `reset_after=5` ⇒ reset only >250 ms behind; 261 ms drift ≈ 211 ms behind ⇒ `overdue_by` returned `None` and each late tick woke instantly while `deadline += period`, shrinking the gap ~one period per tick.
- A1.2 done: `LATE_TICK_MS = 20` in `metrics.rs`; `TickMetrics.late_ticks`/`late_max_ms`; one `warn!` per tick that woke >20 ms late (tick, late_ms, uptime_ms, players; chunk-batch flag omitted — not easily available); `Scheduler::player_count()`.
- Next: A1.3 experiments Э1/Э2(/Э3) on an in-process server + `parity::Client` (join → accept_teleportation → wait 290 chunks → idle 10 s), 3 runs each, min/median/max into `C:\MineRust-data\traces\m43-jitter.md`.
- Then A1.4 (replay path vs Э2), A1.5, then A2 pool (`GameConfig.workers`, `ServerConfig.workers`, barrier, tests а–г).
- Gate: fmt/clippy/test green at A1.2. Tests: net 21 passed / 1 ignored.