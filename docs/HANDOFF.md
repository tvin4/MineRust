# HANDOFF (state for the next session)

M4.3 **part B in progress** (B1.1 done; B1.2–B4 pending). Part A is done and pushed.
- B1.1: `net::game::grid::RegionGrid` (chunk→region square tile of `region_side` chunks, floored division, tile packed into i32) + `GameConfig.region_side` / `ServerConfig.region_side` (default **0 = no split**, whole world = region 0) + `server.example.toml`. Test: `region_side=0` → any position region 0; side=2 splits at block 32.
- Live behaviour unchanged while `region_side=0` (region 0 only; tick inline when one region). The grid is not used by the tick yet.
- B1.2–B1.5: `Region.players` is a `BTreeMap` (deterministic order); `Transfer {player,from,to,seq}` + `AppliedMove.seq` + `Kick.region`; `Region::tick(tick,now,grid)` reports transfers; `Scheduler` owner index routes join/ack/quit and applies transfers after the barrier in `(from,seq)` order; `TickOutcome::transcript` + `kind="transfer"` replay line `{player,from,to}`. Test: id order whatever the insertion order.
- B2: scenario tests а–д in `net::game::region_tests` (socket-free, 2-chunk tiles, K=0/1/4): boundary crossing continuous, four players cross one tick in `(from,seq)` order with the same transcript at every K, Quit routed to the current owner, unconfirmed player never transferred (confirmation survives), five players × ten ticks with per-tick conservation.
- Next (B3): canonical transcript + replay tests а–в (run twice byte-identical, K=0/1/4/8 equal, journal playback reproduces the run) in `region_tests`.
- Then B4 (docs `ARCH-M4 §3/§6`, `DECISIONS`, `LIVE-CHECK-M4.3B`, `PROGRESS`) + push.
- Gate green at B2: net 64 passed / 4 ignored, server 30; clippy `-D warnings` and fmt clean. Commits: B1 `92f4219`+`87a3c81`.
