# Graph Report - MineRust  (2026-11-07)

## Corpus Check
- 19 files · ~8,132 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 214 nodes · 430 edges · 14 communities
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 13 edges (avg confidence: 0.8)
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
- MineRust
- main.rs
- PROGRESS.md — прогресс проекта

## God Nodes (most connected - your core abstractions)
1. `encode_frame()` - 15 edges
2. `split_frame()` - 13 edges
3. `read_frame()` - 11 edges
4. `ARCHITECTURE.md — многопоточный Minecraft на Rust` - 11 edges
5. `ConnectionError` - 10 edges
6. `CodecError` - 10 edges
7. `handle_connection()` - 9 edges
8. `status_state()` - 9 edges
9. `decode_varint()` - 9 edges
10. `PacketReader<'a>` - 9 edges

## Surprising Connections (you probably didn't know these)
- `all_next_states_roundtrip()` --calls--> `split_frame()`  [INFERRED]
  crates/protocol/src/packets/handshake.rs → crates/protocol/src/frame.rs
- `handshake_roundtrips_through_a_frame()` --calls--> `split_frame()`  [INFERRED]
  crates/protocol/src/packets/handshake.rs → crates/protocol/src/frame.rs
- `ping_and_pong_roundtrip_payloads()` --calls--> `split_frame()`  [INFERRED]
  crates/protocol/src/packets/status.rs → crates/protocol/src/frame.rs
- `status_request_frame_has_no_fields()` --calls--> `split_frame()`  [INFERRED]
  crates/protocol/src/packets/status.rs → crates/protocol/src/frame.rs
- `status_response_roundtrips_unicode_json()` --calls--> `split_frame()`  [INFERRED]
  crates/protocol/src/packets/status.rs → crates/protocol/src/frame.rs

## Import Cycles
- None detected.

## Communities (14 total, 0 thin omitted)

### Community 0 - "ARCHITECTURE.md — многопоточный Minecraft на Rust"
Cohesion: 0.19
Nodes (14): ping_and_pong_roundtrip_payloads(), PingRequest, PongResponse, CodecError, PacketReader, Result, Self, String (+6 more)

### Community 1 - "AGENTS.md — правила для ИИ-агента (opencode)"
Cohesion: 0.25
Nodes (7): AGENTS.md — правила для ИИ-агента (opencode), Главные правила, Если застрял, Как ориентироваться в проекте (graphify), Стиль кода, Тесты, Юридические границы (строго)

### Community 2 - "PROGRESS.md — прогресс проекта"
Cohesion: 0.17
Nodes (11): 10. Риски, 1. Цель, 2. Принципы, 3. Структура workspace, 4. Модель потоков (ядро проекта), 5. Данные и совместимость, 6. Клиент (после сервера), 7. Лаунчер (+3 more)

### Community 3 - "MineRust"
Cohesion: 0.17
Nodes (19): Arc, CodecError, handle_connection(), R, Result, W, serve(), status_state() (+11 more)

### Community 4 - "lib.rs"
Cohesion: 0.23
Nodes (18): check_frame_len(), decode_frame(), decode_frame_len(), empty_payload_frame(), encode_frame(), frame_roundtrip(), length_field_over_three_bytes_is_rejected(), maximum_frame_length_is_accepted() (+10 more)

### Community 5 - "CodecError"
Cohesion: 0.13
Nodes (21): CodecError, decode_varint(), encode_varint(), fixed_width_integers_are_big_endian(), PacketReader, PacketReader<'a>, PacketWriter, reader_rejects_negative_length_prefix() (+13 more)

### Community 7 - "handshake.rs"
Cohesion: 0.22
Nodes (14): all_next_states_roundtrip(), Handshake, handshake_roundtrips_through_a_frame(), handshake_wire_layout_is_length_prefixed(), NextState, overlong_server_address_is_rejected(), CodecError, PacketReader (+6 more)

### Community 8 - "config.rs"
Cohesion: 0.17
Nodes (19): AsRef, ConfigError, example_toml(), load_config(), parses_a_valid_config(), rejects_empty_listen_address(), rejects_non_positive_max_players(), rejects_unknown_fields() (+11 more)

### Community 9 - ".to_json"
Cohesion: 0.53
Nodes (4): json_escapes_special_characters(), json_exposes_version_players_and_motd(), String, StatusInfo

### Community 10 - "status.rs"
Cohesion: 0.33
Nodes (14): handshake(), login_state_is_closed_without_a_response(), read_until_close(), recv(), recv_frame(), StatusInfo, send(), server_list_ping_end_to_end() (+6 more)

### Community 11 - "MineRust"
Cohesion: 0.22
Nodes (8): Building, Goals, Layout, Legal, License, MineRust, Running the server, Status

### Community 12 - "main.rs"
Cohesion: 0.52
Nodes (6): main(), parse_args(), Result, try_main(), ExitCode, PathBuf

### Community 13 - "PROGRESS.md — прогресс проекта"
Cohesion: 0.33
Nodes (5): M0 — Каркас, PROGRESS.md — прогресс проекта, Известные проблемы / блокеры, Что дальше, Что сделано

## Knowledge Gaps
- **27 isolated node(s):** `Status`, `Goals`, `Layout`, `Building`, `Running the server` (+22 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `ConnectionError` connect `MineRust` to `config.rs`?**
  _High betweenness centrality (0.128) - this node is a cross-community bridge._
- **Are the 5 inferred relationships involving `encode_frame()` (e.g. with `.to_frame()` and `.to_frame()`) actually correct?**
  _`encode_frame()` has 5 INFERRED edges - model-reasoned connections that need verification._
- **Are the 5 inferred relationships involving `split_frame()` (e.g. with `all_next_states_roundtrip()` and `handshake_roundtrips_through_a_frame()`) actually correct?**
  _`split_frame()` has 5 INFERRED edges - model-reasoned connections that need verification._
- **Are the 2 inferred relationships involving `read_frame()` (e.g. with `handle_connection()` and `status_state()`) actually correct?**
  _`read_frame()` has 2 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Status`, `Goals`, `Layout` to the rest of the system?**
  _27 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `CodecError` be split into smaller, more focused modules?**
  _Cohesion score 0.12660028449502134 - nodes in this community are weakly interconnected._