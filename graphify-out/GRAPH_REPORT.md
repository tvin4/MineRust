# Graph Report - MineRust  (2026-11-07)

## Corpus Check
- 7 files · ~3,958 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 88 nodes · 158 edges · 7 communities
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 2 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `574422e5`
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

## God Nodes (most connected - your core abstractions)
1. `CodecError` - 14 edges
2. `ARCHITECTURE.md — многопоточный Minecraft на Rust` - 11 edges
3. `decode_varint()` - 10 edges
4. `PacketReader<'a>` - 9 edges
5. `PacketWriter` - 9 edges
6. `encode_frame()` - 9 edges
7. `encode_varint()` - 7 edges
8. `check_frame_len()` - 7 edges
9. `AGENTS.md — правила для ИИ-агента (opencode)` - 7 edges
10. `MineRust` - 7 edges

## Surprising Connections (you probably didn't know these)
- `encode_frame()` --calls--> `encode_varint()`  [INFERRED]
  crates/protocol/src/frame.rs → crates/protocol/src/codec.rs
- `decode_frame_len()` --calls--> `decode_varint()`  [INFERRED]
  crates/protocol/src/frame.rs → crates/protocol/src/codec.rs
- `check_frame_len()` --references--> `CodecError`  [EXTRACTED]
  crates/protocol/src/frame.rs → crates/protocol/src/codec.rs
- `decode_frame()` --references--> `CodecError`  [EXTRACTED]
  crates/protocol/src/frame.rs → crates/protocol/src/codec.rs
- `decode_frame_len()` --references--> `CodecError`  [EXTRACTED]
  crates/protocol/src/frame.rs → crates/protocol/src/codec.rs

## Import Cycles
- None detected.

## Communities (7 total, 0 thin omitted)

### Community 0 - "ARCHITECTURE.md — многопоточный Minecraft на Rust"
Cohesion: 0.18
Nodes (11): 10. Риски, 1. Цель, 2. Принципы, 3. Структура workspace, 4. Модель потоков (ядро проекта), 5. Данные и совместимость, 6. Клиент (после сервера), 7. Лаунчер (+3 more)

### Community 1 - "AGENTS.md — правила для ИИ-агента (opencode)"
Cohesion: 0.25
Nodes (7): AGENTS.md — правила для ИИ-агента (opencode), Главные правила, Если застрял, Как ориентироваться в проекте (graphify), Стиль кода, Тесты, Юридические границы (строго)

### Community 2 - "PROGRESS.md — прогресс проекта"
Cohesion: 0.13
Nodes (12): M0 — Каркас, PROGRESS.md — прогресс проекта, Известные проблемы / блокеры, Что дальше, Что сделано, Building, Goals, Layout (+4 more)

### Community 3 - "MineRust"
Cohesion: 0.25
Nodes (11): decode_varint(), reader_rejects_negative_length_prefix(), reader_reports_trailing_bytes(), string_length_counts_utf16_code_units(), string_rejects_invalid_utf8(), varint_accepts_non_canonical_encoding(), varint_matches_documented_encodings(), varint_rejects_overlong_input() (+3 more)

### Community 4 - "lib.rs"
Cohesion: 0.23
Nodes (14): PacketReader, check_frame_len(), decode_frame(), decode_frame_len(), empty_payload_frame(), encode_frame(), frame_roundtrip(), length_field_over_three_bytes_is_rejected() (+6 more)

### Community 5 - "CodecError"
Cohesion: 0.33
Nodes (5): CodecError, PacketReader<'a>, Result, String, Utf8Error

### Community 6 - "PacketWriter"
Cohesion: 0.31
Nodes (5): encode_varint(), fixed_width_integers_are_big_endian(), PacketWriter, Vec, string_roundtrips_unicode()

## Knowledge Gaps
- **25 isolated node(s):** `Главные правила`, `Юридические границы (строго)`, `Стиль кода`, `Как ориентироваться в проекте (graphify)`, `Тесты` (+20 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `CodecError` connect `CodecError` to `MineRust`, `lib.rs`, `PacketWriter`?**
  _High betweenness centrality (0.101) - this node is a cross-community bridge._
- **Why does `ARCHITECTURE.md — многопоточный Minecraft на Rust` connect `ARCHITECTURE.md — многопоточный Minecraft на Rust` to `PROGRESS.md — прогресс проекта`?**
  _High betweenness centrality (0.052) - this node is a cross-community bridge._
- **Why does `encode_frame()` connect `lib.rs` to `CodecError`, `PacketWriter`?**
  _High betweenness centrality (0.041) - this node is a cross-community bridge._
- **What connects `Главные правила`, `Юридические границы (строго)`, `Стиль кода` to the rest of the system?**
  _25 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `PROGRESS.md — прогресс проекта` be split into smaller, more focused modules?**
  _Cohesion score 0.13333333333333333 - nodes in this community are weakly interconnected._