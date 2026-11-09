# HANDOFF (state for the next session)

M4.2 done (`ca8d310`): the game tick applies player movement.
- Connection classifies move packets (`MoveKind {Pos,PosRot,Rot,StatusOnly}`) and stores the raw payload in `MoveSlot`; the tick decodes it (`net::game::movement`) and applies it only after `TeleportAck` (`PlayerPos`, `Player::apply_move`).
- `Region::tick`/`Scheduler::tick` return `TickOutcome {moves, kicks}`; `record_moves()` writes the `move` replay line (x/y/z/yaw/pitch/on_ground).
- Tests: net 20 passed / 1 ignored (decode per kind, ignore before ack, apply after ack, rotation keeps position, replay determinism).
- Automated e2e (one-off smoke, not in repo): in-process server plus a hand-driven `parity::Client` gives a replay of 21 lines, x from 96.7 to 100.3 by 0.2, one move per tick, no jumps. Owner's F3 check: `docs/LIVE-CHECK-M4.2.md`.
- Deferred: `set_chunk_cache_center` moves to M4.4 (streaming). Next: M4.3 (parallel core) is NOT started; wait for an explicit "M4.3 можно".
