# HANDOFF (state for the next session)
Round 5 (M4.0 (а) acceptance): ЧАСТЬ 0 (`733caf9`) + ЧАСТЬ 1 (this commit).
- Part 0: `tools/trace-analyze/analyze.py` (aggregates only, `--list` for
  lists); regression tests for `Watchdog` and the `forceload query` detector.
- Part 1: the world criterion is now the radius-10 send window;
  `tests/window_probe.rs` re-runs П2/П3 (`MINECRAFT_WINDOW_PROBE=p2|p3`) and
  keeps the world in `C:\MineRust-data\world-window-p2|p3`.
  Outside the window 141/279 new chunks, all `structure_starts`, 21-25 chunks
  from the last centre; inside 0 new, `initialize_light->full` 21/41, and
  heightmap/light rewrite of 27/33 already-full chunks (vanilla load/save).
- Facts: `C:\MineRust-data\traces\facts.md`; scans `window-*-scan.txt`.
Next: ЧАСТЬ 2 — the 3682 ms batch gap in П3; then П4/П5 on v3 (Part 3).
Server crates (`server`, `net`, `world`, `protocol`) untouched.