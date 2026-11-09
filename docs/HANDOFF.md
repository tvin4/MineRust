# HANDOFF (state for the next session)
Round 4 (M4.0 walk-trace), parity only; ЧАСТИ I (d10e7ec), II (7ee7874), III (4941cc1), IV.
- I: Э1 kicked → refuted; Э2 passed → Э3 skipped. Rule: a move needs >=1
  `client_tick_end` since the previous move, else vanilla kicks
  `Invalid move player packet received`. П2 302 moves/x 156.9, П3 427/
  (156.9, 28.9); world STOP = vanilla radius-23 generation (hypothesis, 0 in controls).
- II: metrics-only bot. Ours metrics ~5.48 s vs JSONL ~10.78 s (bot-limited,
  confirmed); vanilla ~5.57 vs ~6.03-6.56; not-read share ~82-84% vs ~18-27%.
- III: `build_template_v3.rs` → `world-template-v3`; forceload limit 256/cmd,
  `Watchdog`, 3721/3721 full in 156 s, 61.12 MB, ready zone 65x65, 1584 diffs.
- IV (this commit): `proxy.rs` + bin `mcproxy` + `proxy_verify`; standing bot
  10 s: 14287 rows, handshake/login/configuration/play both ways, monotonic
  `read_ms`/`write_ms`, no compression (`login_compression`/`hello` → abort).
Next: ЧАСТЬ V (ARCH-M4 §6, TICK-MEASURE, debts), then the single final report.
Data: C:\MineRust-data\traces\ (part1-4 reports, proxy-conn-*.jsonl).