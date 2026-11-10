# HANDOFF (state for the next session)

M4.3 **part A done** (part B waits for "M4.3 B можно").
- A1.1 (code): drift 261→1.8 was sequential catch-up, not a reset — `overdue_by` only resets past 250 ms (`reset_after=5`); 261 ms drift ≈ 211 ms behind.
- A1.2 `c45c3b9`: `LATE_TICK_MS=20`, `TickMetrics.late_ticks/late_max_ms`, one `warn` per late tick, `Scheduler::player_count()`.
- A1.3/A1.4: Э1/Э2 ×3 (in-process server + hand-driven `parity::Client`, 289 chunks, 10 s idle, release) → **no tick >20 ms late** (jitter max ≤1.34 ms); Э3 not run (Э1 clean), replay left synchronous ("source of 261 ms not established"). Report: `C:\MineRust-data\traces\m43-jitter.md`.
- A2: `net::game::pool` (K std::threads, ownership via mpsc, ≤2 s stop) + `Scheduler` barrier merging by `region_id`, inline when ≤1 region; `GameConfig.workers` (0 = off), `ServerConfig.workers` (default cores−2, 0 = off), `net::serve_with`.
- Tests: K=0/1/4 equal outcome (8 regions), order independence, pool stop <2 s, 1000 idle pooled ticks drift 0.597 ms / p99 1.030 ms; one region inline p50 0.10 µs vs dispatch p50 0.70 µs (hypothesis to M4.6).
- Live check for owner: `docs/LIVE-CHECK-M4.3A.md`. Not started: Transfer, second region, outgoing replay test (part B).