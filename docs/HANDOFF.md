# HANDOFF (state for the next session)

M4.2 done (`ca8d310`, `a994d23`): the game tick applies player movement.
- Connection classifies move packets (`MoveKind {Pos,PosRot,Rot,StatusOnly}`) and stores the raw payload in `MoveSlot`; the tick decodes it (`net::game::movement`) and applies it only after `TeleportAck` (`PlayerPos`, `Player::apply_move`).
- `Region::tick`/`Scheduler::tick` return `TickOutcome {moves, kicks}`; `record_moves()` writes the `move` replay line (x/y/z/yaw/pitch/on_ground/flags).
- The trailing byte of every move packet is a flags byte (`a994d23`): bit 0 is on-ground, 0x00..0x03 are all valid, bit 1's meaning is not established (kept raw as `flags`, never dropped).
- Tests: net 21 passed / 1 ignored (decode per kind, 0x00..=0x03 for pos/pos_rot, ignore before ack, apply after ack, rotation keeps position, replay determinism).
- Automated e2e (one-off smoke, not in repo): in-process server + hand-driven `parity::Client` → replay 21 lines, x 96.7→100.3 by 0.2, one move per tick. Owner's F3 check: `docs/LIVE-CHECK-M4.2.md`.
- Deferred: `set_chunk_cache_center` → M4.4 (streaming). Next: M4.3 (parallel core) — NOT started, wait for an explicit "M4.3 можно".
