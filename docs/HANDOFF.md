# HANDOFF (state for the next session)

Round 6 ЧАСТЬ 1 done (`280b752`): window shape measured (473, half-width 11,
shift 23/37). Part 2 in progress: M4.1.

- ЧАСТЬ 2а (this commit): M4.1 plan (10 lines) written in `ARCH-M4.md` §6:
  `net::game` module (messages, `Region`, trivial `Scheduler`, game thread on
  an absolute 20 TPS deadline, tick-duration p50/p99/max, kick on outbound
  overflow, JSONL replay). Order б→в→г→д; chunks stay in `batch.rs`, join
  packets unchanged.
- Next: ЧАСТЬ 2б — messages, `Region`, scheduler, game thread + 1000-tick
  idle test (drift < 5 ms, p99 < 20 ms). Server crates change only from here.
