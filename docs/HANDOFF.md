# HANDOFF (state for the next session)

## Done — M4.0 (а) round 2 and round 3 part I.1 (070783e..ec6f12b)
- Round 2 accepted: tool fixes, P1-P3, ЧАСТЬ III table, final report given.
- Round 3,I.1: first/last bot move_player_pos (25 B, 3 BE doubles + 1 byte,
  tails 0x00, all finite) and three live-client packets decoded; kick facts
  in `C:\MineRust-data\traces\part1-move-bytes.md` (no disconnect packet in
  the trace: os error 10054; vanilla log line kept in run2-vanilla-latest).

## Next
- Owner reviews; server code untouched (M4 plan still not accepted).
- Pitfall: bot `PROBE_WAIT` adds ~3000 ms of self-wait to our series.