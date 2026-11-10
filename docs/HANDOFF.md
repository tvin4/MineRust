# HANDOFF (state for the next session)

M4.3 **part B in progress** (B1.1 done; B1.2–B4 pending). Part A is done and pushed.
- B1.1: `net::game::grid::RegionGrid` (chunk→region square tile of `region_side` chunks, floored division, tile packed into i32) + `GameConfig.region_side` / `ServerConfig.region_side` (default **0 = no split**, whole world = region 0) + `server.example.toml`. Test: `region_side=0` → any position region 0; side=2 splits at block 32.
- Live behaviour unchanged while `region_side=0` (region 0 only; tick inline when one region). The grid is not used by the tick yet.
- Next (B1.2): BTreeMap players (deterministic order), `Transfer {player,from,to,seq}` + `AppliedMove.seq` + `Kick.region`, scheduler `owners` index + routing Join/TeleportAck/Quit/Move, post-barrier transfer application, `kind="transfer"` replay line, determinism test.
- Then B2 (scenario tests а–д), B3 (canonical transcript + replay tests а–в), B4 (docs + `docs/LIVE-CHECK-M4.3B.md`).
- Gate green at B1.1: net 57 passed / 4 ignored, server 30; clippy `-D warnings` and fmt clean.
