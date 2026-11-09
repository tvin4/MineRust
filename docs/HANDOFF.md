# HANDOFF (state for the next session)

## Done
- Part1 C1–C3 (349eb74, 77e7ea2, 780add6): §0 channel semantics, overloaded-tick
  policy + tick-duration metric, §3 rewrite (tokio drift «не объяснён», load
  excluded, priority hypothesis in M4.6).
- Part1 C4 (e551428): §1 replay — nondeterminism sources (time, keep-alive
  ids, UUID, inter-task order) listed as recorded inputs.

## In progress
- Part1 C5: «Что даёт параллелизм» paragraph in §1/§5 (region parallelism
  only with spread players; real multicore win now = chunk load/encoding).

## Next
- C6 position unchecked + TeleportAck gate → §0/M4.2 + DECISIONS.md.
- C7 ECS deferred → DECISIONS.md. Then Part 2: M4.0 plan ≤12 lines → СТОП.