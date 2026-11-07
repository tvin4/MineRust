# Graph Report - MineRust  (2026-11-07)

## Corpus Check
- 50 files · ~45,228 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 861 nodes · 2187 edges · 29 communities
- Extraction: 95% EXTRACTED · 5% INFERRED · 0% AMBIGUOUS · INFERRED: 120 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `703f95c9`
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
- PacketKey
- status.rs
- encode_frame
- PacketIds
- trace_inspect.rs
- frame.rs
- Value
- configuration.rs
- Uuid
- Client
- Trace
- Iterator
- status.rs
- SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3)

## God Nodes (most connected - your core abstractions)
1. `CodecError` - 90 edges
2. `encode_frame()` - 34 edges
3. `PacketIds` - 33 edges
4. `NbtError` - 30 edges
5. `PacketReader` - 28 edges
6. `DataError` - 27 edges
7. `split_frame()` - 26 edges
8. `Trace` - 25 edges
9. `Client` - 23 edges
10. `GameData` - 22 edges

## Surprising Connections (you probably didn't know these)
- `load()` --calls--> `jar_with()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/fixtures.rs
- `load_from_jar()` --calls--> `load_registries_report()`  [INFERRED]
  crates/data/src/load.rs → crates/data/src/registry_ids.rs
- `load_the_real_client_jar()` --calls--> `load_from_jar()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/load.rs
- `a_registry_without_entries_is_an_error()` --calls--> `load_from_zip()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/load.rs
- `load()` --calls--> `load_from_zip()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/load.rs

## Import Cycles
- None detected.

## Communities (29 total, 0 thin omitted)

### Community 0 - "ARCHITECTURE.md — многопоточный Minecraft на Rust"
Cohesion: 0.20
Nodes (17): Child, copy_dir_all(), copy_dir_all_copies_nested_trees(), copy_dir_all_missing_source_is_an_error(), prepare_world(), Duration, Error, Option (+9 more)

### Community 1 - "AGENTS.md — правила для ИИ-агента (opencode)"
Cohesion: 0.22
Nodes (8): AGENTS.md — правила для ИИ-агента (opencode), Главные правила, Если застрял, Источники истины (порядок), Как ориентироваться в проекте (graphify), Стиль кода, Тесты, Юридические границы (строго)

### Community 2 - "PROGRESS.md — прогресс проекта"
Cohesion: 0.06
Nodes (30): 10. Риски, 1. Цель, 2. Принципы, 3. Структура workspace, 4. Модель потоков (ядро проекта), 5. Данные и совместимость, 6. Клиент (после сервера), 7. Лаунчер (+22 more)

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
Nodes (38): ConfigError, example_toml(), load_config(), parses_a_valid_config(), parses_vanilla_paths(), registries_report_path_is_optional(), rejects_an_empty_client_jar_path(), rejects_an_empty_registries_report_path() (+30 more)

### Community 9 - "login.rs"
Cohesion: 0.12
Nodes (23): disconnect_reason_carries_a_json_text_component(), DisconnectLogin, GameProfile, login_acknowledged_has_no_fields(), login_start_rejects_an_oversized_name(), login_start_roundtrips_name_and_uuid(), login_success_places_the_session_id_last(), login_success_roundtrips_a_profile_with_properties() (+15 more)

### Community 10 - "status.rs"
Cohesion: 0.19
Nodes (20): handshake(), read_until_close(), recv(), recv_frame(), Arc, SocketAddr, TcpStream, Vec (+12 more)

### Community 11 - "NbtError"
Cohesion: 0.17
Nodes (19): packets_from_json(), empty_tags_of_synchronized_registries_are_kept(), every_synchronized_registry_is_loaded(), load(), missing_optional_references_are_skipped(), missing_required_references_are_an_error(), nested_registry_files_are_rejected(), nested_tag_files_use_the_full_tag_path() (+11 more)

### Community 12 - "main.rs"
Cohesion: 0.20
Nodes (13): load_registries_report(), ProtocolId, registries_from_json_named(), RegistryIds, RegistryReport, AsRef, BTreeMap, Item (+5 more)

### Community 13 - "PacketKey"
Cohesion: 0.12
Nodes (14): packets_report_json(), String, Option, describe_shows_state_direction_and_name(), Direction, every_state_and_direction_has_a_distinct_index(), keys_of_the_same_packet_in_two_directions_differ(), PacketKey (+6 more)

### Community 14 - "status.rs"
Cohesion: 0.31
Nodes (15): collect_values(), qualify(), RawTag, resolve(), resolve_tag(), resolve_value(), BTreeMap, BTreeSet (+7 more)

### Community 15 - "encode_frame"
Cohesion: 0.23
Nodes (19): collect_tags(), load_from_jar(), load_from_zip(), load_registry(), read_json(), read_text(), AsRef, BTreeMap (+11 more)

### Community 16 - "PacketIds"
Cohesion: 0.23
Nodes (13): jar_exactly(), jar_with(), Vec, write_jar(), registries_from_json(), a_registry_without_entries_is_an_error(), empty_registries_report_is_rejected(), registries_report_parses_protocol_ids() (+5 more)

### Community 17 - "trace_inspect.rs"
Cohesion: 0.48
Nodes (6): main(), parse_args(), PathBuf, Result, try_main(), ExitCode

### Community 18 - "frame.rs"
Cohesion: 0.06
Nodes (55): GameData, RegistryData, RegistryEntry, BTreeMap, Item, Iterator, NbtTag, Option (+47 more)

### Community 19 - "Value"
Cohesion: 0.50
Nodes (4): DataError, Error, String, ZipError

### Community 21 - "configuration.rs"
Cohesion: 0.07
Nodes (51): decode(), our_update_tags_equals_the_vanilla_packet(), registry_ids(), report_diffs(), Option, PathBuf, String, Vec (+43 more)

### Community 22 - "Uuid"
Cohesion: 0.10
Nodes (28): PacketReader, confirm_teleportation_roundtrips_the_echo(), ConfirmTeleportation, DeathLocation, game_event_carries_an_ubyte_and_a_float(), GameEvent, keep_alive_roundtrips_its_id(), KeepAlive (+20 more)

### Community 23 - "Client"
Cohesion: 0.14
Nodes (25): build_frame(), Client, ClientError, packets_at_the_threshold_are_zlib_compressed(), play_keep_alive_is_acknowledged(), read_frame(), read_varint_prefixed(), Arc (+17 more)

### Community 24 - "Trace"
Cohesion: 0.06
Nodes (60): count_differences_are_reported_per_packet_kind(), CountRow, diff_traces(), direction_name(), entry(), group(), handshake_port_difference_is_normalised_away(), hex_prefix() (+52 more)

### Community 26 - "Iterator"
Cohesion: 0.08
Nodes (39): Box, EnvError, load_parity_env(), ParityEnv, parse_server_port(), Error, Option, PathBuf (+31 more)

### Community 29 - "status.rs"
Cohesion: 0.18
Nodes (12): ping_and_pong_roundtrip_payloads(), PingRequest, PongResponse, Result, Self, String, Vec, status_request_frame_has_no_fields() (+4 more)

### Community 44 - "SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3)"
Cohesion: 0.22
Nodes (8): SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3), Как проверить (команды), План: крейт `parity` (шаг (2) порядка источников), Пункты статуса, Таблица 1 — пакеты (все ID M1–M2), Таблица 2 — кодеки и константы фрейминга, Таблица 3 — поведение M1–M2, Что было непроверяемо раньше и стало проверяемым

## Knowledge Gaps
- **40 isolated node(s):** `Главные правила`, `Юридические границы (строго)`, `Стиль кода`, `Как ориентироваться в проекте (graphify)`, `Источники истины (порядок)` (+35 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `CodecError` connect `CodecError` to `lib.rs`, `handshake.rs`, `login.rs`, `frame.rs`, `configuration.rs`, `Uuid`, `Client`, `status.rs`?**
  _High betweenness centrality (0.324) - this node is a cross-community bridge._
- **Why does `NbtError` connect `lib.rs` to `MineRust`, `Value`, `CodecError`?**
  _High betweenness centrality (0.229) - this node is a cross-community bridge._
- **Why does `DataError` connect `Value` to `lib.rs`, `config.rs`, `NbtError`, `main.rs`, `PacketKey`, `status.rs`, `encode_frame`, `PacketIds`, `frame.rs`, `Iterator`?**
  _High betweenness centrality (0.198) - this node is a cross-community bridge._
- **Are the 24 inferred relationships involving `encode_frame()` (e.g. with `encode_varint()` and `.to_frame()`) actually correct?**
  _`encode_frame()` has 24 INFERRED edges - model-reasoned connections that need verification._
- **What connects `Главные правила`, `Юридические границы (строго)`, `Стиль кода` to the rest of the system?**
  _40 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `PROGRESS.md — прогресс проекта` be split into smaller, more focused modules?**
  _Cohesion score 0.06060606060606061 - nodes in this community are weakly interconnected._
- **Should `lib.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.1111111111111111 - nodes in this community are weakly interconnected._