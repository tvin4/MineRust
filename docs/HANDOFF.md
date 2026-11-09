# HANDOFF (state for the next session)
Round 5 (M4.0 (а) acceptance) — ЧАСТЬ 0 done in this commit.
- `tools/trace-analyze/analyze.py`: one Python analyzer, aggregates only
  (`summary` / `window` / `gap` / `outside`), raw lists only with `--list`;
  reproduces П3: 137 batches, 535 chunks, 62 forgets, max batch gap 3682 ms.
- Regression tests (only what broke): `Watchdog` sets `max-tick-time=-1` and
  restores `server.properties` verbatim; the `forceload query` empty detector
  accepts the real vanilla 26.3 line. Both in `tests/build_template_v3.rs`.
- Facts: `C:\MineRust-data\traces\facts.md`, every line marked [трейс]/[гипотеза].
Next: ЧАСТЬ 1 — the new world criterion (radius-10 send window); re-run П2/П3
because their worlds are gone, then ЧАСТИ 2–4. Server crates untouched.