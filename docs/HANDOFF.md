# HANDOFF (state for the next session)

## Done
- Part1 C1 (349eb74): §0 send().await/try_recv, Move slot, overflow = count.
- Part1 C2 (77e7ea2): overloaded-tick policy (no catch-up, log, reset after
  N ticks [гипотеза]) + tick duration p50/p99/max in M4.1/M4.6 criteria.
- Part1 C3 (780add6): §3 — load scenario marked non-representative, tokio
  drift «не объяснён», baseline hypothesis checked against ticking code
  (base set after immediate first tick, 1000 periods) = not the cause,
  M4.6 + raised tick-thread priority hypothesis.

## In progress
- Part1 C4: §1 replay — nondeterminism sources (time, keep-alive ids, UUID,
  inter-task message order) fed as recorded inputs.

## Next
- C5 «Что даёт параллелизм» §1/§5, C6 unchecked position + TeleportAck gate
  ({§0, M4.2} + DECISIONS.md), C7 ECS deferred → DECISIONS.md. Then Part 2
  M4.0 plan (≤12 lines) → СТОП.