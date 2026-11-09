# HANDOFF (state for the next session)
Round 5 (M4.0 (а) acceptance): ЧАСТЬ 0 (`733caf9`) + ЧАСТЬ 1 (`ea7e408`) + ЧАСТЬ 2 (this commit).
- Part 0: `tools/trace-analyze/analyze.py` (aggregates only, `--list`); tests
  for `Watchdog` and the `forceload query` detector.
- Part 1: world criterion = radius-10 send window; `window_probe.rs` re-ran
  П2/П3 and kept the worlds. Outside: 141/279 new chunks, all
  `structure_starts`, 21-25 from the last centre; inside: 0 new,
  `initialize_light->full` 21/41, heightmap/light rewrite of 27/33 full chunks.
- Part 2: П3's 3682 ms gap (16847.4->20529.5) is neither generation nor our
  reading: 0 batches awaited ack, acks 0.2-0.3 ms, bot kept ticking; the bot
  stayed in chunk (8,0), so the centre never moved. The gap closes with the
  centre move to (9,1) + 37 forgets, then 4 chunks in ~155 ms. Centre == the
  bot's current chunk (4 observations).
- Facts: `C:\MineRust-data\traces\facts.md`.
Next: ЧАСТЬ 3 — П4/П5 on a fresh v3 copy (walk 120 blocks); then ЧАСТЬ 4 docs.
Server crates (`server`, `net`, `world`, `protocol`) untouched.
