# HANDOFF (state for the next session)

## Done
- B: §6 table re-checked (c951efa). Draft leftovers (c3f5163).
- C: v2 re-checked on `.mca` (729 = 625+104, bbox 27×27; full-only 25×25,
  100-chunk trip impossible); M4.4 criterion split (v2 walk / v3 or
  synthetic 100+), synthetic allowed in §4+§5, v3 size 25–30 MB [гипотеза]
  (5a60af8).
- D: M4.5 real packet names from `reports/packets.json` play.clientbound
  (add/remove entity, player_info_update/remove, move_entity_pos/pos_rot/rot,
  rotate_head, teleport_entity; no `move_entity` w/o suffix).

## In progress
- (none)

## Next
- E: §7 → decisions D1–D8. F: risks (a) static split vs window crossing
  borders, (b) side 64 = whole template in one region. A: 2 tickbench re-runs
  → §3 table (verify tokio -7ms drift baseline), .gitignore for
  tools/tickbench/target. G: final names/hypotheses/fmt/clippy/test/push.