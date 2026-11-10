# HANDOFF (state for the next session)

M4.3 **done** (parts A and B). Commits: `92f4219`, `87a3c81`, `86d9f75`, `b6fd13b`. M4.4 not started.
- B1: `net::game::grid::RegionGrid` (tile of `region_side` chunks, floored, packed i32) + `GameConfig.region_side`/`ServerConfig.region_side` (default **0 = no split**, whole world = region 0, live server unchanged).
- B1: `Scheduler` holds `player_id → region_id`, routes Join/TeleportAck/Quit, applies `Transfer` after the barrier in `(from,seq)` (whole `Player` by ownership); `Region.players` is a `BTreeMap`; replay line `kind="transfer"` `{player,from,to}`; `TickOutcome::transcript` = tick results in `(tick,region,seq)` order (real packets are M4.4).
- B2/B3 tests: `net::game::region_tests` (scenarios а–д, K=0/1/4), `net::game::replay_tests` (transcript twice byte-identical, K=0/1/4/8 equal, journal playback of input lines reproduces the run at K=0/1/4 without a format change), shared `test_support`. Gate green: net 67 passed / 4 ignored, server 30; clippy `-D warnings` + fmt clean.
- Owner live checks 10.11.2026 in `DECISIONS.md` / `ARCH-M4.md` §6: move flags (499: 120 `flags=2`, 66 `flags=3`; **correlation only**, bit 1 **not established**); tick lateness (view 32, send 10, `late_ticks=0`, p99 ≈1.1 ms; 288 ms of 09.11 did not recur, cause **not established**).
- Next: **M4.4** (chunk streaming: cache index, window, batch, forget, `StreamChunks`); the byte-for-byte outgoing check arrives there. `region_side` value + parallelism gain stay hypotheses until M4.6 (D3). Owner checklist: `docs/LIVE-CHECK-M4.3B.md`.
