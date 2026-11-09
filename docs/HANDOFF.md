# HANDOFF (state for the next session)

## Done
- Design partially accepted; M4.0 ok, M4.1+ needs all part 1 fixes + owner OK.
- Part1 C1 (349eb74): §0 net→tick — Join/Quit/TeleportAck via send().await +
  tick try_recv, Move = «last position» slot, overflow = message count,
  chunks bypass queue via batch.rs. tickbench sources committed (4b7d040).

## In progress
- Part1 C2: overloaded-tick policy (no catch-up burst, log, deadline reset
  after N ticks [гипотеза]) + tick duration p50/p99/max metric in M4.1/M4.6.

## Next
- Part1 C3–C7: §3 rewrite (tokio drift «не объяснён» + baseline hypothesis,
  load scenario excluded from D4, thread-priority hypothesis in M4.6),
  replay nondeterminism inputs, «Что даёт параллелизм», position unchecked
  + TeleportAck gate → DECISIONS, ECS deferred → DECISIONS.
- Then Part 2: M4.0 plan (walk trace, v3 template, ≤12 lines) → СТОП.