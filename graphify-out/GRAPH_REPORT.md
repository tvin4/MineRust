# Graph Report - MineRust  (2026-11-07)

## Corpus Check
- 15 files · ~6,494 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 174 nodes · 344 edges · 9 communities
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 13 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `11e1615a`
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
- .to_json

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

## Communities (9 total, 0 thin omitted)

### Community 0 - "ARCHITECTURE.md — многопоточный Minecraft на Rust"
Cohesion: 0.19
Nodes (14): ping_and_pong_roundtrip_payloads(), PingRequest, PongResponse, CodecError, PacketReader, Result, Self, String (+6 more)

### Community 1 - "AGENTS.md — правила для ИИ-агента (opencode)"
Cohesion: 0.25
Nodes (7): AGENTS.md — правила для ИИ-агента (opencode), Главные правила, Если застрял, Как ориентироваться в проекте (graphify), Стиль кода, Тесты, Юридические границы (строго)

### Community 2 - "PROGRESS.md — прогресс проекта"
Cohesion: 0.08
Nodes (23): 10. Риски, 1. Цель, 2. Принципы, 3. Структура workspace, 4. Модель потоков (ядро проекта), 5. Данные и совместимость, 6. Клиент (после сервера), 7. Лаунчер (+15 more)

### Community 3 - "MineRust"
Cohesion: 0.14
Nodes (23): Arc, CodecError, handle_connection(), R, Result, W, serve(), status_state() (+15 more)

### Community 4 - "lib.rs"
Cohesion: 0.23
Nodes (18): check_frame_len(), decode_frame(), decode_frame_len(), empty_payload_frame(), encode_frame(), frame_roundtrip(), length_field_over_three_bytes_is_rejected(), maximum_frame_length_is_accepted() (+10 more)

### Community 5 - "CodecError"
Cohesion: 0.12
Nodes (22): CodecError, decode_varint(), encode_varint(), fixed_width_integers_are_big_endian(), PacketReader, PacketReader<'a>, PacketWriter, reader_rejects_negative_length_prefix() (+14 more)

### Community 7 - "handshake.rs"
Cohesion: 0.22
Nodes (14): all_next_states_roundtrip(), Handshake, handshake_roundtrips_through_a_frame(), handshake_wire_layout_is_length_prefixed(), NextState, overlong_server_address_is_rejected(), CodecError, PacketReader (+6 more)

### Community 9 - ".to_json"
Cohesion: 0.53
Nodes (4): json_escapes_special_characters(), json_exposes_version_players_and_motd(), String, StatusInfo

## Knowledge Gaps
- **26 isolated node(s):** `PacketReader`, `Главные правила`, `Юридические границы (строго)`, `Стиль кода`, `Как ориентироваться в проекте (graphify)` (+21 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `encode_frame()` connect `lib.rs` to `ARCHITECTURE.md — многопоточный Minecraft на Rust`, `handshake.rs`?**
  _High betweenness centrality (0.064) - this node is a cross-community bridge._
- **Why does `split_frame()` connect `lib.rs` to `ARCHITECTURE.md — многопоточный Minecraft на Rust`, `handshake.rs`?**
  _High betweenness centrality (0.030) - this node is a cross-community bridge._
- **Are the 5 inferred relationships involving `encode_frame()` (e.g. with `.to_frame()` and `.to_frame()`) actually correct?**
  _`encode_frame()` has 5 INFERRED edges - model-reasoned connections that need verification._
- **Are the 5 inferred relationships involving `split_frame()` (e.g. with `all_next_states_roundtrip()` and `handshake_roundtrips_through_a_frame()`) actually correct?**
  _`split_frame()` has 5 INFERRED edges - model-reasoned connections that need verification._
- **Are the 2 inferred relationships involving `read_frame()` (e.g. with `handle_connection()` and `status_state()`) actually correct?**
  _`read_frame()` has 2 INFERRED edges - model-reasoned connections that need verification._
- **What connects `PacketReader`, `Главные правила`, `Юридические границы (строго)` to the rest of the system?**
  _26 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `PROGRESS.md — прогресс проекта` be split into smaller, more focused modules?**
  _Cohesion score 0.07692307692307693 - nodes in this community are weakly interconnected._