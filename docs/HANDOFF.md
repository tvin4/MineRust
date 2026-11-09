# HANDOFF (state for the next session)

## Done
- Owner's round-2 orders accepted: ЧАСТЬ I (tool fixes), II (П1-П3 fresh
  v2), III (entry benchmarks vanilla vs ours), ONE final report at the end.
- П1 (round 1) stopped on 10053 + region SHA change; artifacts in
  C:\MineRust-data\traces\ (walk-p1.jsonl, live-seq-51-91.txt).
- П.1 done (775e4f9-fix, feat commit): every packet stamped Instant since
  connect at socket read/write (Client::stamps ↔ trace.packets), JSONL
  t_ms and Batch/CacheMove take the wire stamp; report header says so.

## Next
- П.2: client_tick_end id 13 every 50 ms after first teleport ack, by
  absolute deadline; player_loaded at ack +1001 ms per live seq 91.
- Then П.3 stop-after-quit, П.4 gamemode from trace (drop execute probes),
  П.5 chunk_status.py zone criterion, run П1-П3, ЧАСТЬ III, final report.