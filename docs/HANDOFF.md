# HANDOFF (state for the next session)

Round 6: ЧАСТЬ 1 done (`280b752`), M4.1 plan (`d5f65c5`). Part 2 in progress.

- ЧАСТЬ 2б (this commit): new `net::game` module — messages (`ToGame`,
  `ServerMessage`, `PlayerConnection`, `KickFlag`), single `Region` + `Player`
  + `MoveSlot`, trivial `Scheduler`, `GameHandle`/`GameThread` and the 20 TPS
  loop on an absolute deadline (`sleep` to `start + i·50 ms`, `try_recv`).
  Tick metrics p50/p99/max + drift. Not wired to connections yet.
  - Numbers (`--release --ignored`): 1000 idle ticks — drift **0.0 ms**,
    jitter p50 0.38 / p99 1.03 / max 1.11 ms, duration p50 0.001 / p99 0.002 /
    max 0.043 ms. The 1000-tick test is `#[ignore]` (D6: CI smoke only).
- Next: ЧАСТЬ 2в — Join/TeleportAck/Quit channels + Move slot wired into
  `play_state` (join packets unchanged).
