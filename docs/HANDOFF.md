# HANDOFF (state for the next session)

## Done
- B: §6 table re-checked (c951efa); draft leftovers committed (c3f5163).
- C: v2 re-checked by `chunk_status.py scan` on `.mca` — 729 ready
  (625 full + 104 initialize_light), bbox 27×27, full-only 625 bbox 25×25;
  100-chunk trip on v2 impossible — confirmed. Doc: v2/v3 data paragraph,
  v3 size 25–30 MB [гипотеза], M4.4 criterion = walk on v2 + 100+ on
  v3/synthetic, synthetic provider allowed in §4 and §5.

## In progress
- (none)

## Next
- D: M4.5 packet names from reports/packets.json (no `move_entity` w/o suffix).
- E: §7 → decisions D1–D8. F: risks (a) static region split vs window
  crossing borders, (b) side 64 puts template in one region. A: tickbench
  re-runs (2×) for §3, add .gitignore for tools/tickbench/target. G: final check.