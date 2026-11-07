# Graph Report - MineRust  (2026-11-07)

## Corpus Check
- 37 files · ~32,666 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 634 nodes · 1585 edges · 23 communities (20 shown, 3 thin omitted)
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 86 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `f0267dd2`
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
- login.rs
- status.rs
- NbtError
- main.rs
- AsRef
- Path
- encode_frame
- Arc
- PacketKey
- frame.rs
- configuration.rs
- Uuid
- SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3)

## God Nodes (most connected - your core abstractions)
1. `CodecError` - 88 edges
2. `encode_frame()` - 35 edges
3. `NbtError` - 30 edges
4. `PacketReader` - 28 edges
5. `split_frame()` - 26 edges
6. `DataError` - 21 edges
7. `GameData` - 19 edges
8. `PacketIds` - 18 edges
9. `PacketReader<'a>` - 18 edges
10. `PacketWriter` - 18 edges

## Surprising Connections (you probably didn't know these)
- `load_the_real_client_jar()` --calls--> `load_from_jar()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/load.rs
- `a_registry_without_entries_is_an_error()` --calls--> `load_from_zip()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/load.rs
- `load()` --calls--> `load_from_zip()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/load.rs
- `compiled_packet_ids_match_the_real_report()` --calls--> `load_packets_report()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/packets.rs
- `report_ids_override_the_compiled_defaults()` --calls--> `packets_from_json()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/packets.rs

## Import Cycles
- None detected.

## Communities (23 total, 3 thin omitted)

### Community 0 - "ARCHITECTURE.md — многопоточный Minecraft на Rust"
Cohesion: 0.18
Nodes (12): ping_and_pong_roundtrip_payloads(), PingRequest, PongResponse, Result, Self, String, Vec, status_request_frame_has_no_fields() (+4 more)

### Community 1 - "AGENTS.md — правила для ИИ-агента (opencode)"
Cohesion: 0.22
Nodes (8): AGENTS.md — правила для ИИ-агента (opencode), Главные правила, Если застрял, Источники истины (порядок), Как ориентироваться в проекте (graphify), Стиль кода, Тесты, Юридические границы (строго)

### Community 2 - "PROGRESS.md — прогресс проекта"
Cohesion: 0.06
Nodes (28): 10. Риски, 1. Цель, 2. Принципы, 3. Структура workspace, 4. Модель потоков (ядро проекта), 5. Данные и совместимость, 6. Клиент (после сервера), 7. Лаунчер (+20 more)

### Community 3 - "MineRust"
Cohesion: 0.16
Nodes (18): array_to_tag(), arrays_with_mixed_element_types_are_rejected(), collect_bytes(), collect_ints(), collect_longs(), converted_tags_roundtrip_through_the_network_codec(), dimension_type_style_object_converts(), float_or_double() (+10 more)

### Community 4 - "lib.rs"
Cohesion: 0.11
Nodes (32): Cursor<'a>, decode_reports_consumed_bytes(), depth_limit_allows_512_and_rejects_513(), empty_lists_accept_any_element_type(), every_tag_type_roundtrips(), lengths_beyond_the_input_are_rejected(), NbtTag, negative_array_lengths_are_rejected() (+24 more)

### Community 5 - "CodecError"
Cohesion: 0.08
Nodes (27): CodecError, decode_varint(), encode_varint(), fixed_width_integers_are_big_endian(), ints_and_floats_are_big_endian(), nbt_roundtrips_through_the_packet_cursor(), PacketReader<'a>, PacketWriter (+19 more)

### Community 7 - "handshake.rs"
Cohesion: 0.26
Nodes (12): all_next_states_roundtrip(), Handshake, handshake_roundtrips_through_a_frame(), handshake_wire_layout_is_length_prefixed(), NextState, overlong_server_address_is_rejected(), Result, Self (+4 more)

### Community 8 - "config.rs"
Cohesion: 0.10
Nodes (40): Arc, AsRef, ConfigError, example_toml(), load_config(), parses_a_valid_config(), parses_vanilla_paths(), rejects_an_empty_client_jar_path() (+32 more)

### Community 9 - "login.rs"
Cohesion: 0.12
Nodes (23): disconnect_reason_carries_a_json_text_component(), DisconnectLogin, GameProfile, login_acknowledged_has_no_fields(), login_start_rejects_an_oversized_name(), login_start_roundtrips_name_and_uuid(), login_success_places_the_session_id_last(), login_success_roundtrips_a_profile_with_properties() (+15 more)

### Community 10 - "status.rs"
Cohesion: 0.19
Nodes (20): handshake(), read_until_close(), recv(), recv_frame(), Arc, TcpStream, Vec, send() (+12 more)

### Community 11 - "NbtError"
Cohesion: 0.13
Nodes (36): DataError, Error, String, collect_tags(), load_from_jar(), load_from_zip(), load_registry(), read_json() (+28 more)

### Community 12 - "main.rs"
Cohesion: 0.52
Nodes (6): main(), parse_args(), Result, try_main(), ExitCode, PathBuf

### Community 15 - "encode_frame"
Cohesion: 0.14
Nodes (24): jar_exactly(), jar_with(), Vec, write_jar(), packets_from_json(), a_registry_without_entries_is_an_error(), every_synchronized_registry_is_loaded(), load() (+16 more)

### Community 17 - "PacketKey"
Cohesion: 0.14
Nodes (14): packets_report_json(), String, describe_shows_state_direction_and_name(), Direction, every_state_and_direction_has_a_distinct_index(), keys_of_the_same_packet_in_two_directions_differ(), PacketKey, ProtocolState (+6 more)

### Community 18 - "frame.rs"
Cohesion: 0.06
Nodes (57): GameData, RegistryData, RegistryEntry, BTreeMap, NbtTag, Option, String, Vec (+49 more)

### Community 21 - "configuration.rs"
Cohesion: 0.09
Nodes (39): check_frame_len(), decode_frame_len(), empty_payload_frame(), encode_frame(), frame_roundtrip(), length_field_over_three_bytes_is_rejected(), maximum_frame_length_is_accepted(), negative_frame_length_is_rejected() (+31 more)

### Community 22 - "Uuid"
Cohesion: 0.10
Nodes (28): PacketReader, confirm_teleportation_roundtrips_the_echo(), ConfirmTeleportation, DeathLocation, game_event_carries_an_ubyte_and_a_float(), GameEvent, keep_alive_roundtrips_its_id(), KeepAlive (+20 more)

### Community 44 - "SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3)"
Cohesion: 0.22
Nodes (8): SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3), Как проверить (команды), План: крейт `parity` (шаг (2) порядка источников), Пункты статуса, Таблица 1 — пакеты (все ID M1–M2), Таблица 2 — кодеки и константы фрейминга, Таблица 3 — поведение M1–M2, Что было непроверяемо раньше и стало проверяемым

## Knowledge Gaps
- **38 isolated node(s):** `M0 — Каркас`, `M1 — Handshake и status`, `M2 — Login и вход в пустой мир`, `Пред-M3 аудит источников данных (выполнен)`, `M3, шаг 0: окружение ванильного сервера (выполнено)` (+33 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **3 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `CodecError` connect `CodecError` to `ARCHITECTURE.md — многопоточный Minecraft на Rust`, `lib.rs`, `handshake.rs`, `login.rs`, `frame.rs`, `configuration.rs`, `Uuid`?**
  _High betweenness centrality (0.378) - this node is a cross-community bridge._
- **Why does `NbtError` connect `lib.rs` to `MineRust`, `NbtError`, `CodecError`?**
  _High betweenness centrality (0.278) - this node is a cross-community bridge._
- **Why does `DataError` connect `NbtError` to `frame.rs`, `lib.rs`, `encode_frame`?**
  _High betweenness centrality (0.170) - this node is a cross-community bridge._
- **Are the 25 inferred relationships involving `encode_frame()` (e.g. with `write_frame()` and `encode_varint()`) actually correct?**
  _`encode_frame()` has 25 INFERRED edges - model-reasoned connections that need verification._
- **Are the 18 inferred relationships involving `split_frame()` (e.g. with `decode_varint()` and `client_information_roundtrips_every_field()`) actually correct?**
  _`split_frame()` has 18 INFERRED edges - model-reasoned connections that need verification._
- **What connects `M0 — Каркас`, `M1 — Handshake и status`, `M2 — Login и вход в пустой мир` to the rest of the system?**
  _38 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `PROGRESS.md — прогресс проекта` be split into smaller, more focused modules?**
  _Cohesion score 0.06451612903225806 - nodes in this community are weakly interconnected._