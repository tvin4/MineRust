# HANDOFF (state for the next session)

## Done
- ЧАСТЬ I п.1-5: п.5 = world criterion `chunk_status.py compare`, stop only
  for new chunks outside x -7..19 / z -15..11 (this commit). Checks green.

## Next
- Runs П1-П3: `cargo test -p parity --test walk_trace -- --ignored --nocapture`
  (copy latest.log lines before each run, reports go to traces, П2/П3 auto).
- Then ЧАСТЬ III (173/441 + timings A/B/В) and one final report. Context:
  docs/ROUND2-CONTEXT.md.