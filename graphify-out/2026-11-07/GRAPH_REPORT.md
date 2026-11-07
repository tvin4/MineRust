# Graph Report - MineRust  (2026-11-07)

## Corpus Check
- 45 files · ~39,864 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 795 nodes · 1958 edges · 35 communities (26 shown, 9 thin omitted)
- Extraction: 96% EXTRACTED · 4% INFERRED · 0% AMBIGUOUS · INFERRED: 81 edges (avg confidence: 0.8)
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
- AsRef
- Path
- encode_frame
- Arc
- PacketKey
- frame.rs
- vanilla.rs
- configuration.rs
- Uuid
- Client
- Trace
- String
- encode_frame
- play.rs
- CodecError
- Direction
- Error
- ProtocolState
- NbtTag
- SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3)

## God Nodes (most connected - your core abstractions)
1. `CodecError` - 74 edges
2. `encode_frame()` - 30 edges
3. `NbtError` - 30 edges
4. `Client` - 23 edges
5. `PacketReader` - 22 edges
6. `DataError` - 21 edges
7. `split_frame()` - 20 edges
8. `GameData` - 19 edges
9. `ClientError` - 18 edges
10. `PacketIds` - 18 edges

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

## Communities (35 total, 9 thin omitted)

### Community 0 - "ARCHITECTURE.md — многопоточный Minecraft на Rust"
Cohesion: 0.18
Nodes (12): ping_and_pong_roundtrip_payloads(), PingRequest, PongResponse, Result, Self, String, Vec, status_request_frame_has_no_fields() (+4 more)

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
Cohesion: 0.18
Nodes (17): all_next_states_roundtrip(), Handshake, handshake_roundtrips_through_a_frame(), handshake_wire_layout_is_length_prefixed(), NextState, overlong_server_address_is_rejected(), Result, Self (+9 more)

### Community 8 - "config.rs"
Cohesion: 0.09
Nodes (39): Arc, AsRef, ConfigError, example_toml(), load_config(), parses_a_valid_config(), parses_vanilla_paths(), rejects_an_empty_client_jar_path() (+31 more)

### Community 9 - "login.rs"
Cohesion: 0.13
Nodes (23): PacketReader, split_frame(), disconnect_reason_carries_a_json_text_component(), DisconnectLogin, GameProfile, login_acknowledged_has_no_fields(), login_start_rejects_an_oversized_name(), login_start_roundtrips_name_and_uuid() (+15 more)

### Community 10 - "status.rs"
Cohesion: 0.19
Nodes (20): handshake(), read_until_close(), recv(), recv_frame(), Arc, TcpStream, Vec, send() (+12 more)

### Community 11 - "NbtError"
Cohesion: 0.14
Nodes (35): DataError, Error, String, collect_tags(), load_from_jar(), load_from_zip(), load_registry(), read_json() (+27 more)

### Community 12 - "main.rs"
Cohesion: 0.18
Nodes (21): assert_joined(), join_slice_records_traces_for_both_servers(), join_steps(), known_packs_reply(), out_dir(), parity_env_or_skip(), record_join(), Arc (+13 more)

### Community 15 - "encode_frame"
Cohesion: 0.12
Nodes (27): jar_exactly(), jar_with(), packets_report_json(), String, Vec, write_jar(), packets_from_json(), a_registry_without_entries_is_an_error() (+19 more)

### Community 17 - "PacketKey"
Cohesion: 0.17
Nodes (9): Option, describe_shows_state_direction_and_name(), Direction, every_state_and_direction_has_a_distinct_index(), keys_of_the_same_packet_in_two_directions_differ(), PacketKey, ProtocolState, Self (+1 more)

### Community 18 - "frame.rs"
Cohesion: 0.06
Nodes (57): GameData, RegistryData, RegistryEntry, BTreeMap, NbtTag, Option, String, Vec (+49 more)

### Community 19 - "vanilla.rs"
Cohesion: 0.07
Nodes (44): AbortHandle, Box, Child, ConfigError, EnvError, load_parity_env(), ParityEnv, parse_server_port() (+36 more)

### Community 21 - "configuration.rs"
Cohesion: 0.14
Nodes (26): client_information_rejects_an_oversized_locale(), client_information_roundtrips_every_field(), ClientInformation, finish_ack_has_no_fields(), finish_configuration_has_no_fields(), FinishAck, FinishConfiguration, read_count() (+18 more)

### Community 22 - "Uuid"
Cohesion: 0.13
Nodes (18): CodecError, ConfirmTeleportation, DeathLocation, GameEvent, KeepAlive, LoginPlay, PlayerInfoEntry, PlayerInfoUpdate (+10 more)

### Community 23 - "Client"
Cohesion: 0.14
Nodes (27): build_frame(), Client, ClientError, packets_at_the_threshold_are_zlib_compressed(), play_keep_alive_is_acknowledged(), read_frame(), read_varint_prefixed(), Arc (+19 more)

### Community 24 - "Trace"
Cohesion: 0.13
Nodes (21): from_hex(), hex_digit(), hex_roundtrips(), HexError, Direction, Error, Option, ProtocolState (+13 more)

### Community 26 - "encode_frame"
Cohesion: 0.25
Nodes (15): check_frame_len(), decode_frame(), decode_frame_len(), empty_payload_frame(), encode_frame(), frame_roundtrip(), length_field_over_three_bytes_is_rejected(), maximum_frame_length_is_accepted() (+7 more)

### Community 27 - "play.rs"
Cohesion: 0.34
Nodes (11): confirm_teleportation_roundtrips_the_echo(), game_event_carries_an_ubyte_and_a_float(), keep_alive_roundtrips_its_id(), login_play_roundtrips_a_death_location(), login_play_roundtrips_all_fields(), player_info_update_rejects_unsupported_actions(), player_info_update_roundtrips_the_join_actions(), set_center_chunk_roundtrips_chunk_coordinates() (+3 more)

### Community 44 - "SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3)"
Cohesion: 0.22
Nodes (8): SOURCES.md — аудит источников данных (M1–M2, по состоянию на пред-M3), Как проверить (команды), План: крейт `parity` (шаг (2) порядка источников), Пункты статуса, Таблица 1 — пакеты (все ID M1–M2), Таблица 2 — кодеки и константы фрейминга, Таблица 3 — поведение M1–M2, Что было непроверяемо раньше и стало проверяемым

## Knowledge Gaps
- **39 isolated node(s):** `M0 — Каркас`, `M1 — Handshake и status`, `M2 — Login и вход в пустой мир`, `Пред-M3 аудит источников данных (выполнен)`, `M3, шаг 0: окружение ванильного сервера (выполнено)` (+34 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **9 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `CodecError` connect `Uuid` to `ARCHITECTURE.md — многопоточный Minecraft на Rust`, `lib.rs`, `CodecError`, `handshake.rs`, `login.rs`, `frame.rs`, `encode_frame`, `play.rs`?**
  _High betweenness centrality (0.230) - this node is a cross-community bridge._
- **Why does `NbtError` connect `lib.rs` to `MineRust`, `NbtError`, `Uuid`?**
  _High betweenness centrality (0.214) - this node is a cross-community bridge._
- **Why does `DataError` connect `NbtError` to `frame.rs`, `lib.rs`, `encode_frame`?**
  _High betweenness centrality (0.174) - this node is a cross-community bridge._
- **Are the 20 inferred relationships involving `encode_frame()` (e.g. with `write_frame()` and `encode_varint()`) actually correct?**
  _`encode_frame()` has 20 INFERRED edges - model-reasoned connections that need verification._
- **What connects `M0 — Каркас`, `M1 — Handshake и status`, `M2 — Login и вход в пустой мир` to the rest of the system?**
  _39 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `PROGRESS.md — прогресс проекта` be split into smaller, more focused modules?**
  _Cohesion score 0.0625 - nodes in this community are weakly interconnected._
- **Should `lib.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.1111111111111111 - nodes in this community are weakly interconnected._