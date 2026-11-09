# HANDOFF (state for the next session)
Round 5 (M4.0 (а) acceptance): ЧАСТИ 0-2 done (`733caf9`,`ea7e408`,`f61c452`); ЧАСТЬ 3 this commit.
- Part 3: `WalkMode::WalkV3` (same scheme as П2/П3) + `in_zone_v3`
  (centre chunk x∈[-14,26], z∈[-22,18]); `Vanilla::start_from` copies an
  explicit template, so `tests/window_probe.rs` can run П4/П5 on a fresh
  `world-template-v3` copy (`MINECRAFT_WINDOW_PROBE=p4|p5`, 600 steps).
- П4 Completed: 600 moves, 8 centres, 138 forgets; П5: 6 centres, 178
  forgets; both spectator, `player_position`=1, no zone-boundary hit.
- Window criterion on v3: 0 new chunks inside/outside, 1/2 full chunks had
  their heightmap/light rewritten, no status change.
- Facts: `C:\MineRust-data\traces\facts.md`; traces `window-p4|p5.jsonl`.
Next: ЧАСТЬ 4 — ARCH-M4 §6/§4 doc updates, close the criterion item, add the
safe-zone run instruction. Server crates untouched.
