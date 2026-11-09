# HANDOFF (state for the next session)

Round 6: ЧАСТЬ 1 (`280b752`), M4.1 plan (`d5f65c5`), 2б (`aa51fec`),
2в (`d85e537`), 2г (`0720599`), 2д (this commit). Part 2 done.

- ЧАСТЬ 2д (this commit): the replay journal (`net::game::replay`,
  `Replay<W: Write>` + `ReplayRecord`). The tick writes one JSONL line
  `{tick, seq, region, src, kind, payload}` at apply time; enabled by
  `MINERUST_REPLAY` (value = file path). `move` kind lands in M4.2.
  Determinism test: join→ack→quit recorded twice is byte-identical.
- M4.1 code (б–д) is complete: tick, region, channels, kick policy,
  overloaded tick, replay. Still M4.2+ (movement, regions, streaming).
- Next: live-check checklist in `docs/` + `tools/run-server*.bat`
  (ЧАСТЬ 3), then the final report.
