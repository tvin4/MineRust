# Graph Report - MineRust  (2026-11-07)

## Corpus Check
- 50 files · ~45,228 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 881 nodes · 2156 edges · 44 communities (30 shown, 14 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 87 edges (avg confidence: 0.8)
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
- Item
- Iterator
- RegistryData
- RegistryEntry
- status.rs
- read_frame
- ParityEnv
- status_slice.rs
- BTreeMap
- Item
- Iterator
- Arc
- R
- Result
- TcpStream
- W
- encode_frame
- SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3)

## God Nodes (most connected - your core abstractions)
1. `CodecError` - 90 edges
2. `encode_frame()` - 34 edges
3. `PacketReader` - 28 edges
4. `NbtError` - 28 edges
5. `DataError` - 27 edges
6. `PacketIds` - 26 edges
7. `split_frame()` - 26 edges
8. `Client` - 23 edges
9. `GameData` - 22 edges
10. `Trace` - 21 edges

## Surprising Connections (you probably didn't know these)
- `load_the_real_client_jar()` --calls--> `load_from_jar()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/load.rs
- `a_registry_without_entries_is_an_error()` --calls--> `load_from_zip()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/load.rs
- `empty_registries_report_is_rejected()` --calls--> `registries_from_json()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/registry_ids.rs
- `registries_report_parses_protocol_ids()` --calls--> `registries_from_json()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/registry_ids.rs
- `registries_report_with_invalid_json_names_the_file()` --calls--> `registries_from_json()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/registry_ids.rs

## Import Cycles
- None detected.

## Communities (44 total, 14 thin omitted)

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
Nodes (24): decode_varint(), encode_varint(), fixed_width_integers_are_big_endian(), ints_and_floats_are_big_endian(), nbt_roundtrips_through_the_packet_cursor(), PacketReader<'a>, PacketWriter, positions_pack_x_z_and_y() (+16 more)

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
Cohesion: 0.17
Nodes (21): json_escapes_special_characters(), json_exposes_version_players_and_motd(), String, StatusInfo, handshake(), read_until_close(), recv(), recv_frame() (+13 more)

### Community 11 - "NbtError"
Cohesion: 0.05
Nodes (73): DataError, Error, String, collect_tags(), load_from_jar(), load_from_zip(), load_registry(), read_json() (+65 more)

### Community 13 - "PacketKey"
Cohesion: 0.12
Nodes (19): jar_exactly(), jar_with(), packets_report_json(), String, Vec, write_jar(), Cursor, describe_shows_state_direction_and_name() (+11 more)

### Community 16 - "PacketIds"
Cohesion: 0.18
Nodes (15): load_packets_report(), PacketIds, packets_from_json(), parse_report(), ProtocolId, ReportFile, AsRef, HashMap (+7 more)

### Community 17 - "trace_inspect.rs"
Cohesion: 0.10
Nodes (33): BTreeMap, print_tables(), prints_registry_and_tag_tables(), tag_counts(), take_string(), trace_dir(), vanilla_registry_ids(), vanilla_tags() (+25 more)

### Community 18 - "frame.rs"
Cohesion: 0.11
Nodes (30): Arc, ConnectionError, GameData, RegistryData, RegistryEntry, Option, String, UpdateTags (+22 more)

### Community 21 - "configuration.rs"
Cohesion: 0.18
Nodes (12): CodecError, client_information_rejects_an_oversized_locale(), FinishAck, FinishConfiguration, read_count(), registry_data_entry_order_is_the_numeric_id_order(), RegistryData, Result (+4 more)

### Community 22 - "Uuid"
Cohesion: 0.10
Nodes (29): PacketReader, Position, confirm_teleportation_roundtrips_the_echo(), ConfirmTeleportation, DeathLocation, game_event_carries_an_ubyte_and_a_float(), GameEvent, keep_alive_roundtrips_its_id() (+21 more)

### Community 23 - "Client"
Cohesion: 0.16
Nodes (24): build_frame(), Client, ClientError, packets_at_the_threshold_are_zlib_compressed(), play_keep_alive_is_acknowledged(), read_frame(), read_varint_prefixed(), Arc (+16 more)

### Community 24 - "Trace"
Cohesion: 0.08
Nodes (48): count_differences_are_reported_per_packet_kind(), CountRow, diff_traces(), direction_name(), entry(), group(), handshake_port_difference_is_normalised_away(), hex_prefix() (+40 more)

### Community 25 - "Item"
Cohesion: 0.26
Nodes (15): split_frame(), client_information_roundtrips_every_field(), ClientInformation, finish_ack_has_no_fields(), finish_configuration_has_no_fields(), registry_data_roundtrips_entries_with_and_without_data(), RegistryEntry, RegistryTag (+7 more)

### Community 26 - "Iterator"
Cohesion: 0.16
Nodes (13): Box, EnvError, load_parity_env(), parse_server_port(), Error, Option, PathBuf, Result (+5 more)

### Community 29 - "status.rs"
Cohesion: 0.18
Nodes (12): ping_and_pong_roundtrip_payloads(), PingRequest, PongResponse, Result, Self, String, Vec, status_request_frame_has_no_fields() (+4 more)

### Community 30 - "read_frame"
Cohesion: 0.21
Nodes (12): ConnectionError, Error, frame_roundtrips_over_a_stream(), length_field_over_three_bytes_is_rejected(), overlong_length_field_is_rejected(), read_frame(), R, Result (+4 more)

### Community 31 - "ParityEnv"
Cohesion: 0.24
Nodes (15): ParityEnv, assert_joined(), join_slice_records_traces_for_both_servers(), join_steps(), known_packs_reply(), out_dir(), parity_env_or_skip(), record_join() (+7 more)

### Community 32 - "status_slice.rs"
Cohesion: 0.30
Nodes (11): assert_status_trace(), out_dir(), record_status(), AbortHandle, Arc, PathBuf, SocketAddr, Vec (+3 more)

### Community 41 - "encode_frame"
Cohesion: 0.25
Nodes (15): check_frame_len(), decode_frame(), decode_frame_len(), empty_payload_frame(), encode_frame(), frame_roundtrip(), length_field_over_three_bytes_is_rejected(), maximum_frame_length_is_accepted() (+7 more)

### Community 44 - "SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3)"
Cohesion: 0.22
Nodes (8): SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3), Как проверить (команды), План: крейт `parity` (шаг (2) порядка источников), Пункты статуса, Таблица 1 — пакеты (все ID M1–M2), Таблица 2 — кодеки и константы фрейминга, Таблица 3 — поведение M1–M2, Что было непроверяемо раньше и стало проверяемым

## Knowledge Gaps
- **40 isolated node(s):** `M0 — Каркас`, `M1 — Handshake и status`, `M2 — Login и вход в пустой мир`, `Пред-M3 аудит источников данных (выполнен)`, `M3, шаг 0: окружение ванильного сервера (выполнено)` (+35 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **14 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `CodecError` connect `configuration.rs` to `lib.rs`, `CodecError`, `handshake.rs`, `encode_frame`, `login.rs`, `Uuid`, `Client`, `Item`, `status.rs`, `read_frame`?**
  _High betweenness centrality (0.337) - this node is a cross-community bridge._
- **Why does `NbtError` connect `lib.rs` to `MineRust`, `configuration.rs`?**
  _High betweenness centrality (0.149) - this node is a cross-community bridge._
- **Why does `DataError` connect `NbtError` to `PacketIds`, `config.rs`, `Iterator`?**
  _High betweenness centrality (0.148) - this node is a cross-community bridge._
- **Are the 24 inferred relationships involving `encode_frame()` (e.g. with `encode_varint()` and `.to_frame()`) actually correct?**
  _`encode_frame()` has 24 INFERRED edges - model-reasoned connections that need verification._
- **What connects `M0 — Каркас`, `M1 — Handshake и status`, `M2 — Login и вход в пустой мир` to the rest of the system?**
  _40 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `PROGRESS.md — прогресс проекта` be split into smaller, more focused modules?**
  _Cohesion score 0.06060606060606061 - nodes in this community are weakly interconnected._
- **Should `lib.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.1111111111111111 - nodes in this community are weakly interconnected._