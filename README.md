# MineRust

Multithreaded reimplementation of **Minecraft: Java Edition 26.3** in Rust:
a server that vanilla 26.3 clients can join and that reads vanilla worlds, a
Rust client, and a launcher that verifies ownership of the game.

> **Disclaimer**
> Не является официальным продуктом Minecraft, не одобрен Mojang или Microsoft.
> Not an official Minecraft product. Not approved by Mojang or Microsoft.

## Status

Early development. Current milestone: **M0 (skeleton)** — see
[docs/PROGRESS.md](docs/PROGRESS.md) for progress and
[ARCHITECTURE.md](ARCHITECTURE.md) for the full plan (milestones M0–M10).

## Goals

1. **Accuracy**: behaviour matches vanilla, verified automatically by parity
   tests against a vanilla server.
2. **Speed**: ticks, world generation and networking scale across CPU cores.

## Layout

| Path | Purpose |
|------|---------|
| `crates/protocol/` | Packets and codecs (VarInt, NBT, chunk format), version 26.3 |
| `crates/data/` | Registries loaded from the user's own copy of the game (not stored in this repo) |
| `crates/world/` | Chunks, blocks, lighting, Anvil (`.mca`) read/write |
| `crates/worldgen/` | Parallel world generation |
| `crates/ecs/` | Entities and components |
| `crates/sim/` | Tick logic: blocks, physics, mobs |
| `crates/scheduler/` | Regions, thread pool, region boundaries |
| `crates/net/` | tokio networking, packet in/out |
| `crates/server/` | Server binary, config, commands |
| `crates/parity/` | Comparison against the vanilla server |
| `crates/client/` | Client: wgpu + winit (planned) |
| `crates/launcher/` | Tauri launcher (planned) |

Crates are added to the workspace as their milestones start.

## Building

Requires a stable Rust toolchain.

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

CI runs the same checks on every push and pull request
(`.github/workflows/ci.yml`).

## Legal

- No Mojang/Microsoft code, assets or data are stored in this repository.
- Written from observed behaviour and public protocol documentation.
- The launcher verifies game ownership only through the official
  Microsoft/Minecraft services; no authentication bypasses.

## License

[MIT](LICENSE)
