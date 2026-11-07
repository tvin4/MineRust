# Graph Report - MineRust  (2026-11-07)

## Corpus Check
- 37 files · ~32,348 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 661 nodes · 1545 edges · 45 communities (23 shown, 22 thin omitted)
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 54 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `42fbe540`
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
- NbtError
- main.rs
- Result
- login.rs
- encode_frame
- login.rs
- PacketKey
- frame.rs
- Error
- BTreeMap
- configuration.rs
- Uuid
- handle_connection
- .to_json
- R
- W
- AsRef
- Path
- Vec
- Arc
- GameData
- PacketIds
- StatusInfo
- TcpStream
- NbtTag
- CodecError
- PacketReader
- CodecError
- PacketReader
- CodecError
- PacketReader
- Position
- SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3)

## God Nodes (most connected - your core abstractions)
1. `CodecError` - 65 edges
2. `encode_frame()` - 26 edges
3. `DataError` - 21 edges
4. `PacketReader` - 18 edges
5. `PacketReader<'a>` - 18 edges
6. `PacketWriter` - 18 edges
7. `split_frame()` - 17 edges
8. `load()` - 16 edges
9. `handle_connection()` - 14 edges
10. `NbtError` - 14 edges

## Surprising Connections (you probably didn't know these)
- `login_state()` --calls--> `offline_uuid()`  [INFERRED]
  crates/net/src/connection.rs → crates/protocol/src/packets/login.rs
- `play_state()` --calls--> `offline_uuid()`  [INFERRED]
  crates/net/src/connection.rs → crates/protocol/src/packets/login.rs
- `encode_frame()` --calls--> `encode_varint()`  [INFERRED]
  crates/protocol/src/frame.rs → crates/protocol/src/codec.rs
- `read_frame()` --calls--> `decode_varint()`  [INFERRED]
  crates/net/src/frame.rs → crates/protocol/src/codec.rs
- `split_frame()` --calls--> `decode_varint()`  [INFERRED]
  crates/protocol/src/frame.rs → crates/protocol/src/codec.rs

## Import Cycles
- None detected.

## Communities (45 total, 22 thin omitted)

### Community 0 - "ARCHITECTURE.md — многопоточный Minecraft на Rust"
Cohesion: 0.19
Nodes (14): ping_and_pong_roundtrip_payloads(), PingRequest, PongResponse, CodecError, PacketReader, Result, Self, String (+6 more)

### Community 1 - "AGENTS.md — правила для ИИ-агента (opencode)"
Cohesion: 0.22
Nodes (8): AGENTS.md — правила для ИИ-агента (opencode), Главные правила, Если застрял, Источники истины (порядок), Как ориентироваться в проекте (graphify), Стиль кода, Тесты, Юридические границы (строго)

### Community 2 - "PROGRESS.md — прогресс проекта"
Cohesion: 0.07
Nodes (27): 10. Риски, 1. Цель, 2. Принципы, 3. Структура workspace, 4. Модель потоков (ядро проекта), 5. Данные и совместимость, 6. Клиент (после сервера), 7. Лаунчер (+19 more)

### Community 3 - "MineRust"
Cohesion: 0.10
Nodes (34): array_to_tag(), arrays_with_mixed_element_types_are_rejected(), collect_bytes(), collect_ints(), collect_longs(), converted_tags_roundtrip_through_the_network_codec(), dimension_type_style_object_converts(), float_or_double() (+26 more)

### Community 4 - "lib.rs"
Cohesion: 0.20
Nodes (18): Cursor, Cursor<'a>, decode_reports_consumed_bytes(), depth_limit_allows_512_and_rejects_513(), empty_lists_accept_any_element_type(), every_tag_type_roundtrips(), lengths_beyond_the_input_are_rejected(), NbtTag (+10 more)

### Community 5 - "CodecError"
Cohesion: 0.07
Nodes (33): CodecError, decode_varint(), encode_varint(), fixed_width_integers_are_big_endian(), ints_and_floats_are_big_endian(), nbt_roundtrips_through_the_packet_cursor(), PacketReader<'a>, PacketWriter (+25 more)

### Community 7 - "handshake.rs"
Cohesion: 0.26
Nodes (12): all_next_states_roundtrip(), Handshake, handshake_roundtrips_through_a_frame(), handshake_wire_layout_is_length_prefixed(), NextState, overlong_server_address_is_rejected(), Result, Self (+4 more)

### Community 8 - "config.rs"
Cohesion: 0.10
Nodes (37): AsRef, ConfigError, example_toml(), load_config(), parses_a_valid_config(), rejects_an_empty_client_jar_path(), rejects_an_empty_report_path(), rejects_empty_listen_address() (+29 more)

### Community 9 - ".to_json"
Cohesion: 0.23
Nodes (10): BTreeMap, GameData, RegistryData, RegistryEntry, NbtTag, Option, String, Vec (+2 more)

### Community 10 - "status.rs"
Cohesion: 0.16
Nodes (23): handshake(), read_until_close(), recv(), recv_frame(), Arc, GameData, PacketIds, StatusInfo (+15 more)

### Community 11 - "NbtError"
Cohesion: 0.14
Nodes (35): DataError, String, collect_tags(), load_from_jar(), load_from_zip(), load_registry(), read_json(), read_text() (+27 more)

### Community 12 - "main.rs"
Cohesion: 0.52
Nodes (6): main(), parse_args(), Result, try_main(), ExitCode, PathBuf

### Community 15 - "encode_frame"
Cohesion: 0.06
Nodes (39): load_packets_report(), PacketIds, packets_from_json(), parse_report(), ProtocolId, ReportFile, Option, Result (+31 more)

### Community 16 - "login.rs"
Cohesion: 0.10
Nodes (32): empty_payload_frame(), encode_frame(), frame_roundtrip(), negative_packet_id_is_rejected(), oversized_frame_is_rejected(), Vec, split_frame(), split_frame_consumes_length_prefix() (+24 more)

### Community 17 - "PacketKey"
Cohesion: 0.24
Nodes (13): jar_exactly(), jar_with(), packets_report_json(), String, Vec, write_jar(), a_registry_without_entries_is_an_error(), used_keys_are_unique_in_state_direction_and_name() (+5 more)

### Community 18 - "frame.rs"
Cohesion: 0.21
Nodes (12): ConnectionError, Error, frame_roundtrips_over_a_stream(), length_field_over_three_bytes_is_rejected(), overlong_length_field_is_rejected(), read_frame(), R, Result (+4 more)

### Community 21 - "configuration.rs"
Cohesion: 0.14
Nodes (26): client_information_rejects_an_oversized_locale(), client_information_roundtrips_every_field(), ClientInformation, finish_ack_has_no_fields(), finish_configuration_has_no_fields(), FinishAck, FinishConfiguration, read_count() (+18 more)

### Community 22 - "Uuid"
Cohesion: 0.10
Nodes (28): PacketReader, confirm_teleportation_roundtrips_the_echo(), ConfirmTeleportation, DeathLocation, game_event_carries_an_ubyte_and_a_float(), GameEvent, keep_alive_roundtrips_its_id(), KeepAlive (+20 more)

### Community 23 - "handle_connection"
Cohesion: 0.27
Nodes (20): Arc, ConnectionError, configuration_state(), handle_connection(), login_state(), play_state(), Result, String (+12 more)

### Community 24 - ".to_json"
Cohesion: 0.53
Nodes (4): json_escapes_special_characters(), json_exposes_version_players_and_motd(), String, StatusInfo

### Community 44 - "SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3)"
Cohesion: 0.22
Nodes (8): SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3), Как проверить (команды), План: крейт `parity` (шаг (2) порядка источников), Пункты статуса, Таблица 1 — пакеты (все ID M1–M2), Таблица 2 — кодеки и константы фрейминга, Таблица 3 — поведение M1–M2, Что было непроверяемо раньше и стало проверяемым

## Knowledge Gaps
- **38 isolated node(s):** `M0 — Каркас`, `M1 — Handshake и status`, `M2 — Login и вход в пустой мир`, `Пред-M3 аудит источников данных (выполнен)`, `Что дальше` (+33 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **22 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `CodecError` connect `CodecError` to `lib.rs`, `handshake.rs`, `login.rs`, `frame.rs`, `Uuid`, `handle_connection`?**
  _High betweenness centrality (0.227) - this node is a cross-community bridge._
- **Why does `DataError` connect `NbtError` to `lib.rs`, `encode_frame`?**
  _High betweenness centrality (0.149) - this node is a cross-community bridge._
- **Why does `GameData` connect `.to_json` to `status.rs`, `NbtError`, `encode_frame`?**
  _High betweenness centrality (0.086) - this node is a cross-community bridge._
- **Are the 16 inferred relationships involving `encode_frame()` (e.g. with `write_frame()` and `encode_varint()`) actually correct?**
  _`encode_frame()` has 16 INFERRED edges - model-reasoned connections that need verification._
- **What connects `M0 — Каркас`, `M1 — Handshake и status`, `M2 — Login и вход в пустой мир` to the rest of the system?**
  _38 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `PROGRESS.md — прогресс проекта` be split into smaller, more focused modules?**
  _Cohesion score 0.06666666666666667 - nodes in this community are weakly interconnected._
- **Should `MineRust` be split into smaller, more focused modules?**
  _Cohesion score 0.09551020408163265 - nodes in this community are weakly interconnected._