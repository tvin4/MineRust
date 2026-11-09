# HANDOFF (state for the next session)

Round 6: ЧАСТЬ 1 (`280b752`), M4.1 plan (`d5f65c5`), 2б (`aa51fec`),
2в (`d85e537`), 2г (this commit). Part 2 in progress.

- ЧАСТЬ 2г (this commit): the kick policy (`Region::deliver` uses
  `try_send`; `Full` raises `KickFlag::SendQueueOverflow`, removes the
  player and returns a `Kick`; `Closed` does nothing) and the overloaded
  tick (the reset decision is the pure `overdue_by`; the loop resets the
  deadline once instead of replaying missed slots). Unit tests cover both.
- Still no movement parsing/applying (M4.2), no streaming (M4.4).
- Next: ЧАСТЬ 2д — the JSONL replay format `{tick, seq, region, src,
  kind, payload}` written at apply time, enabled by `MINERUST_REPLAY`,
  plus a determinism test (one scenario replayed twice).
