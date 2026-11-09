# HANDOFF (state for the next session)

Round 6: ЧАСТЬ 1 (`280b752`), M4.1 plan (`d5f65c5`), 2б (`aa51fec`). Part 2 in
progress.

- ЧАСТЬ 2в (this commit): `net::serve` starts one detached game thread;
  `play_state` takes a `GameHandle`, uses its id counter for Login (play)
  `entity_id` (first player keeps 1, join bytes unchanged), sends `Join`,
  sends `TeleportAck` on `ConfirmTeleportation`, stores the newest movement
  packet raw in the `MoveSlot`, and a `PlayerGuard` sends `Quit` on exit.
  Added a `ServerMessage` arm to the `select!` (no packets sent in M4.1 yet).
  `play_state` args grouped into `Session` (clippy limit).
- Still no movement parsing/applying (M4.2), no streaming (M4.4).
- Next: ЧАСТЬ 2г — kick policy on outbound overflow + overloaded-tick
  deadline reset + unit tests.
