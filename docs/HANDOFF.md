# HANDOFF (state for the next session)

## Done — round 3 parts I and II.1 (bc72232, 4da8e53)
- I.1/I.2: move_player_pos bytes vs the live client, kick facts, verdict
  "no encoding defect" (`C:\MineRust-data\traces\part1-move-bytes.md`); I.3
  (P2/P3 runs) not executed, owner stop honoured.
- II.1: the scripted known-packs wait is now conditional
  (`PROBE_WAIT` 3 s only for the vanilla port, `PROBE_WAIT_NO_PROBE`
  100 ms otherwise); vanilla series untouched.

## Next
- II.2 re-measure series В (our cold/warm x3) with the new wait, II.3
  payload breakdown, III float paces, IV docs. Server code untouched.