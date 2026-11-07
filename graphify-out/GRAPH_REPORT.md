# Graph Report - MineRust  (2026-11-07)

## Corpus Check
- 45 files · ~39,864 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 772 nodes · 1967 edges · 23 communities
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 96 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `5da7009e`
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
- encode_frame
- frame.rs
- vanilla.rs
- configuration.rs
- Uuid
- Client
- Trace
- SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3)

## God Nodes (most connected - your core abstractions)
1. `CodecError` - 90 edges
2. `encode_frame()` - 34 edges
3. `PacketIds` - 33 edges
4. `NbtError` - 30 edges
5. `PacketReader` - 28 edges
6. `split_frame()` - 26 edges
7. `DataError` - 23 edges
8. `Client` - 23 edges
9. `GameData` - 21 edges
10. `Trace` - 19 edges

## Surprising Connections (you probably didn't know these)
- `load_the_real_client_jar()` --calls--> `load_from_jar()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/load.rs
- `packets_report_json()` --calls--> `used_packet_keys()`  [INFERRED]
  crates/data/src/fixtures.rs → crates/protocol/src/packets/mod.rs
- `a_registry_without_entries_is_an_error()` --calls--> `load_from_zip()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/load.rs
- `load()` --calls--> `load_from_zip()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/load.rs
- `compiled_packet_ids_match_the_real_report()` --calls--> `load_packets_report()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/packets.rs

## Import Cycles
- None detected.

## Communities (23 total, 0 thin omitted)

### Community 0 - "ARCHITECTURE.md — многопоточный Minecraft на Rust"
Cohesion: 0.20
Nodes (17): Child, copy_dir_all(), copy_dir_all_copies_nested_trees(), copy_dir_all_missing_source_is_an_error(), prepare_world(), Duration, Error, Option (+9 more)

### Community 1 - "AGENTS.md — правила для ИИ-агента (opencode)"
Cohesion: 0.22
Nodes (8): AGENTS.md — правила для ИИ-агента (opencode), Главные правила, Если застрял, Источники истины (порядок), Как ориентироваться в проекте (graphify), Стиль кода, Тесты, Юридические границы (строго)

### Community 2 - "PROGRESS.md — прогресс проекта"
Cohesion: 0.06
Nodes (29): 10. Риски, 1. Цель, 2. Принципы, 3. Структура workspace, 4. Модель потоков (ядро проекта), 5. Данные и совместимость, 6. Клиент (после сервера), 7. Лаунчер (+21 more)

### Community 3 - "MineRust"
Cohesion: 0.16
Nodes (18): array_to_tag(), arrays_with_mixed_element_types_are_rejected(), collect_bytes(), collect_ints(), collect_longs(), converted_tags_roundtrip_through_the_network_codec(), dimension_type_style_object_converts(), float_or_double() (+10 more)

### Community 4 - "lib.rs"
Cohesion: 0.11
Nodes (32): Cursor<'a>, decode_reports_consumed_bytes(), depth_limit_allows_512_and_rejects_513(), empty_lists_accept_any_element_type(), every_tag_type_roundtrips(), lengths_beyond_the_input_are_rejected(), NbtTag, negative_array_lengths_are_rejected() (+24 more)

### Community 5 - "CodecError"
Cohesion: 0.08
Nodes (25): decode_varint(), encode_varint(), fixed_width_integers_are_big_endian(), ints_and_floats_are_big_endian(), nbt_roundtrips_through_the_packet_cursor(), PacketReader<'a>, PacketWriter, Position (+17 more)

### Community 7 - "handshake.rs"
Cohesion: 0.26
Nodes (12): all_next_states_roundtrip(), Handshake, handshake_roundtrips_through_a_frame(), handshake_wire_layout_is_length_prefixed(), NextState, overlong_server_address_is_rejected(), Result, Self (+4 more)

### Community 8 - "config.rs"
Cohesion: 0.11
Nodes (36): ConfigError, example_toml(), load_config(), parses_a_valid_config(), parses_vanilla_paths(), rejects_an_empty_client_jar_path(), rejects_an_empty_report_path(), rejects_empty_listen_address() (+28 more)

### Community 9 - "login.rs"
Cohesion: 0.06
Nodes (52): CodecError, check_frame_len(), decode_frame_len(), empty_payload_frame(), encode_frame(), frame_roundtrip(), length_field_over_three_bytes_is_rejected(), maximum_frame_length_is_accepted() (+44 more)

### Community 10 - "status.rs"
Cohesion: 0.11
Nodes (30): GameData, RegistryData, RegistryEntry, BTreeMap, NbtTag, Option, String, Vec (+22 more)

### Community 11 - "NbtError"
Cohesion: 0.14
Nodes (35): DataError, Error, String, collect_tags(), load_from_jar(), load_from_zip(), load_registry(), read_json() (+27 more)

### Community 12 - "main.rs"
Cohesion: 0.52
Nodes (6): main(), parse_args(), PathBuf, Result, try_main(), ExitCode

### Community 15 - "encode_frame"
Cohesion: 0.08
Nodes (40): jar_exactly(), jar_with(), packets_report_json(), String, Vec, write_jar(), load_packets_report(), packets_from_json() (+32 more)

### Community 18 - "frame.rs"
Cohesion: 0.12
Nodes (33): PacketIds, configuration_state(), handle_connection(), login_state(), play_state(), Arc, R, Result (+25 more)

### Community 19 - "vanilla.rs"
Cohesion: 0.08
Nodes (39): Box, EnvError, load_parity_env(), ParityEnv, parse_server_port(), Error, Option, PathBuf (+31 more)

### Community 21 - "configuration.rs"
Cohesion: 0.13
Nodes (24): client_information_rejects_an_oversized_locale(), client_information_roundtrips_every_field(), ClientInformation, finish_ack_has_no_fields(), finish_configuration_has_no_fields(), FinishAck, FinishConfiguration, read_count() (+16 more)

### Community 22 - "Uuid"
Cohesion: 0.10
Nodes (28): PacketReader, confirm_teleportation_roundtrips_the_echo(), ConfirmTeleportation, DeathLocation, game_event_carries_an_ubyte_and_a_float(), GameEvent, keep_alive_roundtrips_its_id(), KeepAlive (+20 more)

### Community 23 - "Client"
Cohesion: 0.16
Nodes (24): build_frame(), Client, ClientError, packets_at_the_threshold_are_zlib_compressed(), play_keep_alive_is_acknowledged(), read_frame(), read_varint_prefixed(), Arc (+16 more)

### Community 24 - "Trace"
Cohesion: 0.06
Nodes (32): Option, from_hex(), hex_digit(), hex_roundtrips(), HexError, Error, Option, Path (+24 more)

### Community 44 - "SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3)"
Cohesion: 0.22
Nodes (8): SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3), Как проверить (команды), План: крейт `parity` (шаг (2) порядка источников), Пункты статуса, Таблица 1 — пакеты (все ID M1–M2), Таблица 2 — кодеки и константы фрейминга, Таблица 3 — поведение M1–M2, Что было непроверяемо раньше и стало проверяемым

## Knowledge Gaps
- **39 isolated node(s):** `Главные правила`, `Юридические границы (строго)`, `Стиль кода`, `Как ориентироваться в проекте (graphify)`, `Источники истины (порядок)` (+34 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `CodecError` connect `login.rs` to `lib.rs`, `CodecError`, `handshake.rs`, `frame.rs`, `configuration.rs`, `Uuid`, `Client`?**
  _High betweenness centrality (0.359) - this node is a cross-community bridge._
- **Why does `NbtError` connect `lib.rs` to `MineRust`, `login.rs`, `NbtError`?**
  _High betweenness centrality (0.242) - this node is a cross-community bridge._
- **Why does `DataError` connect `NbtError` to `lib.rs`, `config.rs`, `encode_frame`, `vanilla.rs`, `Trace`?**
  _High betweenness centrality (0.179) - this node is a cross-community bridge._
- **Are the 24 inferred relationships involving `encode_frame()` (e.g. with `encode_varint()` and `.to_frame()`) actually correct?**
  _`encode_frame()` has 24 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Главные правила`, `Юридические границы (строго)`, `Стиль кода` to the rest of the system?**
  _39 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `PROGRESS.md — прогресс проекта` be split into smaller, more focused modules?**
  _Cohesion score 0.0625 - nodes in this community are weakly interconnected._
- **Should `lib.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.1111111111111111 - nodes in this community are weakly interconnected._