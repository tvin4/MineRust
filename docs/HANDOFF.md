# HANDOFF (state for the next session)
Round 5 (M4.0 (а) acceptance) — all four parts done; this commit is ЧАСТЬ 4.
Commits: `733caf9` (tools), `ea7e408` (window criterion), `f61c452` (П3
pause), `6e271d6` (П4/П5 on v3), ЧАСТЬ 4 (this one).
- Docs: `ARCH-M4.md` §4 fixed (walk trace exists; `forget_level_chunk` = two
  big-endian i32; centre follows the bot's chunk; window square confirmed),
  §6 got "Итоги раунда 5" (decided vs hypothesis), round-4 criterion item
  closed (new chunks outside the radius-10 window are not a violation).
- `docs/LIVE-WALK.md`: how to run a live client on a v3 copy through the
  recording proxy on 25565, with the safe walking zone (chunk x −14..26,
  z −22..18). NOT run; waiting for the owner's decision.
- `PROGRESS.md` got the round-5 journal entry.
- Facts: `C:\MineRust-data\traces\facts.md`. Server crates untouched.
Next: owner decides whether to run the live client; M4.4 streaming work after.
