# Graph Report - MineRust  (2026-11-07)

## Corpus Check
- 19 files · ~8,348 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 209 nodes · 441 edges · 12 communities
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 22 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `a5103b2a`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- ARCHITECTURE.md — многопоточный Minecraft на Rust
- AGENTS.md — правила для ИИ-агента (opencode)
- PROGRESS.md — прогресс проекта
- MineRust
- lib.rs
- CodecError
- handshake.rs
- config.rs
- .to_json
- status.rs
- main.rs

## God Nodes (most connected - your core abstractions)
1. `CodecError` - 30 edges
2. `encode_frame()` - 17 edges
3. `split_frame()` - 14 edges
4. `read_frame()` - 13 edges
5. `decode_varint()` - 12 edges
6. `ARCHITECTURE.md — многопоточный Minecraft на Rust` - 11 edges
7. `handle_connection()` - 10 edges
8. `status_state()` - 10 edges
9. `ConnectionError` - 10 edges
10. `decode_frame()` - 10 edges

## Surprising Connections (you probably didn't know these)
- `handle_connection()` --calls--> `read_frame()`  [INFERRED]
  crates/net/src/connection.rs → crates/net/src/frame.rs
- `handle_connection()` --calls--> `decode_frame()`  [INFERRED]
  crates/net/src/connection.rs → crates/protocol/src/frame.rs
- `status_state()` --calls--> `read_frame()`  [INFERRED]
  crates/net/src/connection.rs → crates/net/src/frame.rs
- `status_state()` --calls--> `write_frame()`  [INFERRED]
  crates/net/src/connection.rs → crates/net/src/frame.rs
- `status_state()` --calls--> `decode_frame()`  [INFERRED]
  crates/net/src/connection.rs → crates/protocol/src/frame.rs

## Import Cycles
- None detected.

## Communities (12 total, 0 thin omitted)

### Community 0 - "ARCHITECTURE.md — многопоточный Minecraft на Rust"
Cohesion: 0.15
Nodes (17): ConnectionError, Error, CodecError, PacketReader, ping_and_pong_roundtrip_payloads(), PingRequest, PongResponse, Result (+9 more)

### Community 1 - "AGENTS.md — правила для ИИ-агента (opencode)"
Cohesion: 0.25
Nodes (7): AGENTS.md — правила для ИИ-агента (opencode), Главные правила, Если застрял, Как ориентироваться в проекте (graphify), Стиль кода, Тесты, Юридические границы (строго)

### Community 2 - "PROGRESS.md — прогресс проекта"
Cohesion: 0.07
Nodes (25): 10. Риски, 1. Цель, 2. Принципы, 3. Структура workspace, 4. Модель потоков (ядро проекта), 5. Данные и совместимость, 6. Клиент (после сервера), 7. Лаунчер (+17 more)

### Community 3 - "MineRust"
Cohesion: 0.29
Nodes (10): frame_roundtrips_over_a_stream(), length_field_over_three_bytes_is_rejected(), overlong_length_field_is_rejected(), read_frame(), R, Result, Vec, W (+2 more)

### Community 4 - "lib.rs"
Cohesion: 0.26
Nodes (16): check_frame_len(), decode_frame(), decode_frame_len(), empty_payload_frame(), encode_frame(), frame_roundtrip(), length_field_over_three_bytes_is_rejected(), maximum_frame_length_is_accepted() (+8 more)

### Community 5 - "CodecError"
Cohesion: 0.12
Nodes (19): decode_varint(), encode_varint(), fixed_width_integers_are_big_endian(), PacketReader<'a>, PacketWriter, reader_rejects_negative_length_prefix(), reader_reports_trailing_bytes(), Result (+11 more)

### Community 7 - "handshake.rs"
Cohesion: 0.24
Nodes (12): all_next_states_roundtrip(), Handshake, handshake_roundtrips_through_a_frame(), handshake_wire_layout_is_length_prefixed(), NextState, overlong_server_address_is_rejected(), Result, Self (+4 more)

### Community 8 - "config.rs"
Cohesion: 0.18
Nodes (18): AsRef, ConfigError, example_toml(), load_config(), parses_a_valid_config(), rejects_empty_listen_address(), rejects_non_positive_max_players(), rejects_unknown_fields() (+10 more)

### Community 9 - ".to_json"
Cohesion: 0.24
Nodes (12): Arc, handle_connection(), R, Result, TcpStream, W, serve(), status_state() (+4 more)

### Community 10 - "status.rs"
Cohesion: 0.37
Nodes (13): handshake(), login_state_is_closed_without_a_response(), read_until_close(), recv(), recv_frame(), TcpStream, Vec, send() (+5 more)

### Community 12 - "main.rs"
Cohesion: 0.52
Nodes (6): main(), parse_args(), Result, try_main(), ExitCode, PathBuf

## Knowledge Gaps
- **27 isolated node(s):** `Главные правила`, `Юридические границы (строго)`, `Стиль кода`, `Как ориентироваться в проекте (graphify)`, `Тесты` (+22 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `CodecError` connect `ARCHITECTURE.md — многопоточный Minecraft на Rust` to `lib.rs`, `CodecError`, `handshake.rs`?**
  _High betweenness centrality (0.249) - this node is a cross-community bridge._
- **Why does `handle_connection()` connect `.to_json` to `ARCHITECTURE.md — многопоточный Minecraft на Rust`, `MineRust`, `lib.rs`?**
  _High betweenness centrality (0.151) - this node is a cross-community bridge._
- **Why does `StatusInfo` connect `.to_json` to `config.rs`, `status.rs`?**
  _High betweenness centrality (0.140) - this node is a cross-community bridge._
- **Are the 7 inferred relationships involving `encode_frame()` (e.g. with `write_frame()` and `encode_varint()`) actually correct?**
  _`encode_frame()` has 7 INFERRED edges - model-reasoned connections that need verification._
- **Are the 6 inferred relationships involving `split_frame()` (e.g. with `decode_varint()` and `all_next_states_roundtrip()`) actually correct?**
  _`split_frame()` has 6 INFERRED edges - model-reasoned connections that need verification._
- **Are the 4 inferred relationships involving `read_frame()` (e.g. with `handle_connection()` and `status_state()`) actually correct?**
  _`read_frame()` has 4 INFERRED edges - model-reasoned connections that need verification._
- **Are the 3 inferred relationships involving `decode_varint()` (e.g. with `read_frame()` and `decode_frame_len()`) actually correct?**
  _`decode_varint()` has 3 INFERRED edges - model-reasoned connections that need verification._