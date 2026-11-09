# HANDOFF (state for the next session)
Round 4 (M4.0 walk-trace), ЧАСТЬ I done and committed (parity only). Э1
(first move after load) kicked → refuted; Э2 (three live moves + 5 s idle)
passed → stop-at-first-success, Э3 not run. Trace rule: a move must follow
>=1 `client_tick_end` since the previous move, else vanilla kicks
`Invalid move player packet received`. П2 302 moves / 23 150 ms / x 156.9,
П3 427 / 30 062 ms / (156.9, 28.9): both Completed, 0 zero-tick moves. World
STOP from an out-of-zone strip = vanilla radius-23 generation around the last
player chunk (hypothesis; controls nobot/stand/walk50 = 0 new) — owner
decision pending. Next: ЧАСТЬ II (metrics-only bot), III (v3), IV, V (docs);
big data: C:\MineRust-data\traces\part1-report.md.