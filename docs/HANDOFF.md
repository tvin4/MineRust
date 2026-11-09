# HANDOFF (state for the next session)

## Done
- Round-2 orders accepted: ЧАСТЬ I-III, ONE final report at the end;
  script only, no M4.1/no server code. Live seq 51-91 in
  traces/live-seq-51-91.txt; ack 84114.5607 → player_loaded 85115.8432.
- П.1 (1d31f17): wire stamp per packet, Instant from connect
  (Client::stamps ↔ trace.packets); JSONL t_ms uses it everywhere.
- П.2 (2d826e2): client_tick_end id 13 every 50 ms on the absolute grid
  anchored by first ack; player_loaded at ack+1001 ms; recv wakes on the
  nearest of movement/tick/loaded deadlines.

## Next
- П.3 server future stops vanilla only after quit; П.4 mode from trace;
  П.5 chunk_status.py zone; then runs П1-П3, ЧАСТЬ III, final report.