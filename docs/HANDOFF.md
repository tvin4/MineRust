# HANDOFF (state for the next session)
Round 4 (M4.0 (а) walk-trace) done, parity only; ЧАСТИ I (d10e7ec), II (7ee7874),
III (4941cc1), IV (5d77c44), V (this commit).
- Decided by trace/measure: a move needs ≥1 `client_tick_end` since the previous
  move, else vanilla kicks `Invalid move player packet received`; window
  `set_chunk_cache_center` moves with the bot; our old entry numbers were
  bot-limited (metrics ~5.48 s vs JSONL ~10.78 s; vanilla ~5.57 vs ~6.03-6.56);
  `forceload add` limit exactly 256/cmd; v3 = 61.12 MB, 4225 `full`, ready zone
  65x65 (target +-2), v2→v3 1584 status diffs incl. exactly 104 light; proxy
  25565→25566 records state/dir/id/base64 + monotonic ms, no compression.
- Hypothesis: world STOP = vanilla radius-23 generation around the last player
  chunk (owner's decision on the criterion still open); flag byte meaning.
- Debts: (а) 24 sky arrays/chunk vs 2-7 vanilla (+17.97 MB/441) — light task or
  separately, not now; (б) entry numbers marked «bot-limited».
Next: owner decides on the STOP criterion and on starting M4.1 (plan said code
only after «план принят»). Data: C:\MineRust-data\traces\ (part1-4, proxy JSONL).