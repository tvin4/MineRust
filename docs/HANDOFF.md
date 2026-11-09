# HANDOFF (state for the next session)

Round 7: ЧАСТЬ 1 — stop the game thread on shutdown (`c7bfa68`); ЧАСТЬ 2 — window docs.
- Game thread no longer detached: `net::serve` = accept loop in `select!` vs `shutdown_signal()` (Ctrl+C; Win Ctrl+Break/close), then `GameThread::join_timeout(2 s)` (stop flag read once per tick).
- Live check OK: Ctrl+Break → "shutdown signal received" → "stop requested, leaving the tick loop" → "game thread stopped", exit 0 (<1 s).
- Docs: §4 window formula verified only at `view_distance=10`; we send 21×21=441 vs vanilla 473, closed in M4.4.
- Next: M4.2 (parse/apply movement), NOT started — waiting for an explicit "M4.2 можно".
