# MineRust

Multithreaded reimplementation of **Minecraft: Java Edition 26.3** in Rust:
a server that vanilla 26.3 clients can join and that reads vanilla worlds, a
Rust client, and a launcher that verifies ownership of the game.

> **Disclaimer**
> Не является официальным продуктом Minecraft, не одобрен Mojang или Microsoft.
> Not an official Minecraft product. Not approved by Mojang or Microsoft.

## Status

Early development. Current milestone: **M1 (handshake & status)** — see
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

## Running the server

1. Copy `server.example.toml` to `server.toml` (it is gitignored) and fill in
   the values from **your own copy of the game**: `protocol_version` and
   `version_name` live in `version.json` inside the client jar, and
   `client_jar` is the path to that jar (registries and tags are loaded from
   `data/minecraft/` inside it at startup). Do not commit real values.

   ```toml
   listen = "0.0.0.0:25565"
   motd = "A MineRust server"
   max_players = 20
   client_jar = "path/to/your/client.jar"
   protocol_version = <from your game's version.json>
   version_name = "<from your game's version.json>"
   ```

2. Start the server:

   ```bash
   cargo run -p server            # uses ./server.toml
   cargo run -p server -- --config path/to/server.toml
   ```

3. Check it in game: launch Minecraft 26.3, open **Multiplayer → Add Server**,
   enter `localhost:25565` — the server list should show the MOTD, the version
   name and `0/20` players.

Milestone M1 covers the handshake/status flow only: the server answers server
list pings and closes the connection on login (M2).

## Legal

- No Mojang/Microsoft code, assets or data are stored in this repository.
- Written from observed behaviour and public protocol documentation.
- The launcher verifies game ownership only through the official
  Microsoft/Minecraft services; no authentication bypasses.

## License

[MIT](LICENSE)
