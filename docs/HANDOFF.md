# HANDOFF (state for the next session)
Round 4 (M4.0 walk-trace), parity only; ЧАСТЬ I committed (d10e7ec).
- I: Э1 kicked → refuted; Э2 (live moves + 5 s idle) passed → Э3 skipped.
  Rule from traces: a move must follow >=1 `client_tick_end`, else vanilla
  kicks `Invalid move player packet received`. П2 302 moves / x 156.9,
  П3 427 / (156.9, 28.9); world STOP = vanilla radius-23 generation around
  the last player chunk (hypothesis; controls nobot/stand/walk50 = 0).
- II (this commit): metrics-only bot mode (no JSONL/base64 live, one JSON
  after exit) + buffered socket; both fold facts through
  `PreloadMetrics::record`. Warm series (25566 / 25565), 4.6715 float, x3
  each: ours metrics ~5.48 s vs JSONL ~10.78 s (bot-limited, hypothesis
  confirmed); vanilla ~5.57 vs ~6.03-6.56; not-read share ~82-84% JSONL vs
  ~18-27% metrics. Old entry numbers now marked «замер ограничен ботом».
Next: ЧАСТЬ III (world-template-v3), IV (proxy), V (docs). Big data:
C:\MineRust-data\traces\ (part1-report.md, part2-report.md).
