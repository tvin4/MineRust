# Graph Report - MineRust  (2026-11-07)

## Corpus Check
- 11 files · ~5,522 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 146 nodes · 296 edges · 10 communities
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 10 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `744d25a1`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- ARCHITECTURE.md — многопоточный Minecraft на Rust
- AGENTS.md — правила для ИИ-агента (opencode)
- PROGRESS.md — прогресс проекта
- MineRust
- lib.rs
- CodecError
- PacketWriter
- handshake.rs
- .new
- .to_json

## God Nodes (most connected - your core abstractions)
1. `encode_frame()` - 15 edges
2. `split_frame()` - 13 edges
3. `ARCHITECTURE.md — многопоточный Minecraft на Rust` - 11 edges
4. `CodecError` - 10 edges
5. `decode_varint()` - 9 edges
6. `PacketReader<'a>` - 9 edges
7. `PacketWriter` - 9 edges
8. `check_frame_len()` - 8 edges
9. `decode_frame()` - 7 edges
10. `Handshake` - 7 edges

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

## Communities (10 total, 0 thin omitted)

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
Cohesion: 0.36
Nodes (8): decode_varint(), encode_varint(), PacketReader, varint_accepts_non_canonical_encoding(), varint_matches_documented_encodings(), varint_rejects_overlong_input(), varint_rejects_truncated_input(), varint_roundtrips_across_the_range()

### Community 4 - "lib.rs"
Cohesion: 0.25
Nodes (18): check_frame_len(), decode_frame(), decode_frame_len(), empty_payload_frame(), encode_frame(), frame_roundtrip(), length_field_over_three_bytes_is_rejected(), maximum_frame_length_is_accepted() (+10 more)

### Community 5 - "CodecError"
Cohesion: 0.38
Nodes (4): CodecError, PacketReader<'a>, Result, Utf8Error

### Community 6 - "PacketWriter"
Cohesion: 0.31
Nodes (4): fixed_width_integers_are_big_endian(), PacketWriter, Vec, string_roundtrips_unicode()

### Community 7 - "handshake.rs"
Cohesion: 0.22
Nodes (14): all_next_states_roundtrip(), Handshake, handshake_roundtrips_through_a_frame(), handshake_wire_layout_is_length_prefixed(), NextState, overlong_server_address_is_rejected(), CodecError, PacketReader (+6 more)

### Community 8 - ".new"
Cohesion: 0.25
Nodes (6): reader_rejects_negative_length_prefix(), reader_reports_trailing_bytes(), string_length_counts_utf16_code_units(), string_rejects_invalid_utf8(), Self, String

### Community 9 - ".to_json"
Cohesion: 0.53
Nodes (4): json_escapes_special_characters(), json_exposes_version_players_and_motd(), String, StatusInfo

## Knowledge Gaps
- **26 isolated node(s):** `PacketReader`, `Главные правила`, `Юридические границы (строго)`, `Стиль кода`, `Как ориентироваться в проекте (graphify)` (+21 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `encode_frame()` connect `lib.rs` to `ARCHITECTURE.md — многопоточный Minecraft на Rust`, `handshake.rs`?**
  _High betweenness centrality (0.091) - this node is a cross-community bridge._
- **Why does `CodecError` connect `CodecError` to `.new`, `MineRust`, `PacketWriter`?**
  _High betweenness centrality (0.054) - this node is a cross-community bridge._
- **Why does `split_frame()` connect `lib.rs` to `ARCHITECTURE.md — многопоточный Minecraft на Rust`, `handshake.rs`?**
  _High betweenness centrality (0.043) - this node is a cross-community bridge._
- **Are the 5 inferred relationships involving `encode_frame()` (e.g. with `.to_frame()` and `.to_frame()`) actually correct?**
  _`encode_frame()` has 5 INFERRED edges - model-reasoned connections that need verification._
- **Are the 5 inferred relationships involving `split_frame()` (e.g. with `all_next_states_roundtrip()` and `handshake_roundtrips_through_a_frame()`) actually correct?**
  _`split_frame()` has 5 INFERRED edges - model-reasoned connections that need verification._
- **What connects `PacketReader`, `Главные правила`, `Юридические границы (строго)` to the rest of the system?**
  _26 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `PROGRESS.md — прогресс проекта` be split into smaller, more focused modules?**
  _Cohesion score 0.07692307692307693 - nodes in this community are weakly interconnected._