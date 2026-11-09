# HANDOFF (state for the next session)

Round 6 done: ЧАСТЬ 1 (`280b752`), M4.1 б–д (`d5f65c5`, `aa51fec`,
`d85e537`, `0720599`, `0bec27f`), ЧАСТЬ 3 (this commit).

- M4.1 code is complete: 20 TPS deadline tick, `HashMap<id, Player>` in one
  `Region`, Join/TeleportAck/Quit over `send().await`, Move slot,
  kick-on-overflow, overloaded-tick reset, JSONL replay (`MINERUST_REPLAY`).
- `tools\run-server.bat` and `tools\run-server-vanilla.bat` added; the
  first was dry-run (listens on 25565, world template untouched).
- Live client check NOT done: `docs/LIVE-CHECK-M4.1.md` is the checklist.
- Next milestone is M4.2 (parse/apply movement), not started.
