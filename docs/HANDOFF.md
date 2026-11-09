# HANDOFF (state for the next session)

## Done
- B: §6 table (c951efa). C: v2 data re-checked, M4.4 split, §4/§5
  synthetic (5a60af8). D: packet names (20a3c16). E: §7 D1–D8 (510d1bb).
- F: §7 «Риски»: (а) static split vs window crossing region borders,
  M5/Folia dynamic merge; (б) side-64 = template in one region,
  parallelism only on synthetic.

## In progress
- (none)

## Next
- A (last): tick measurement for §3 — foreground `cargo run --release`
  (tools/tickbench?, check), 1000 × 50 ms, 2 scenarios (idle/loaded),
  3 methods (absolute deadline sleep / hybrid deadline−1-2ms sleep+spin /
  tokio::interval), output to file, report min/median/p99/p99.9/max;
  re-check past numbers (tokio −7 ms drift suspicious = baseline bug?),
  finalize §3 table + D4, add .gitignore for tools/tickbench/target.
- G: names exist in repo, «гипотеза» markers, fmt/clippy/test, commit, push.