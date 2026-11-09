# HANDOFF (state for the next session)

Round 6, ЧАСТЬ 1 (analysis only, server crates untouched). `analyze.py` got a
`shape` subcommand + `test_analyze.py` (4 tests, OK); fixed the
`forget_level_chunk` decode (packed `ChunkPos`, z first, then x).

- Measured (traces П2/П4/П5): the load window after join is a **23×23 square
  with cut corners**, half-width **11**, **473** chunks (not the ±10 square of
  441). Row widths by |dx|: `0..6→23, 7→21, 8→19, 9→19, 10→15, 11→13`.
- A one-chunk centre shift delivers the leading edge and forgets the trailing
  one; loaded stays 473. П4 (+x) **23/23**, П5 (diagonal) **37/37** — that is
  why 23/37, not 21/41 (21/41 are the ±10 square's edge/corner).
- First П4 shift (t=6442.6) happened mid-join: 0 forgotten, 313→160→473.
  "473 instead of 441" is the window size.
- "Rollback 1" in the П4/П5 tables is the single normal entry teleport:
  `seq=54`, t=1166.5/1405.5, pos (96.5,145.0,−31.5), `teleport_id=1`.
- Documented in `ARCH-M4.md` §4/§6 + D2, `PROGRESS.md` (round 6) and
  `C:\MineRust-data\traces\facts.md`. Window implementation (M4.4) unchanged.
- Hypothesis left: half-width 11 = R+1 and the corner-cut rule.

Next: Round 6 ЧАСТЬ 2 — M4.1 (game tick thread, channels, `Region`, replay).
