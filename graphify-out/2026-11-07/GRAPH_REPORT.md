# Graph Report - MineRust  (2026-11-07)

## Corpus Check
- 33 files · ~25,216 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 559 nodes · 1359 edges · 43 communities (19 shown, 24 thin omitted)
- Extraction: 98% EXTRACTED · 2% INFERRED · 0% AMBIGUOUS · INFERRED: 30 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `eb135145`
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
- config.rs
- .to_json
- status.rs
- NbtError
- main.rs
- .decode_network
- login.rs
- encode_frame
- MineRust
- PROGRESS.md — прогресс проекта
- Error
- Result
- String
- configuration.rs
- Uuid
- .to_json
- Arc
- R
- W
- Self
- String
- Uuid
- Self
- Uuid
- Vec
- BTreeMap
- Option
- Result
- String
- Vec
- Arc
- GameData
- StatusInfo
- TcpStream
- Vec

## God Nodes (most connected - your core abstractions)
1. `CodecError` - 86 edges
2. `PacketReader` - 28 edges
3. `NbtError` - 27 edges
4. `PacketReader<'a>` - 18 edges
5. `PacketWriter` - 18 edges
6. `encode_frame()` - 17 edges
7. `load()` - 16 edges
8. `ConnectionError` - 15 edges
9. `DataError` - 15 edges
10. `split_frame()` - 14 edges

## Surprising Connections (you probably didn't know these)
- `a_registry_without_entries_is_an_error()` --calls--> `jar_exactly()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/fixtures.rs
- `encode_frame()` --calls--> `encode_varint()`  [INFERRED]
  crates/protocol/src/frame.rs → crates/protocol/src/codec.rs
- `read_frame()` --calls--> `decode_varint()`  [INFERRED]
  crates/net/src/frame.rs → crates/protocol/src/codec.rs
- `decode_frame_len()` --calls--> `decode_varint()`  [INFERRED]
  crates/protocol/src/frame.rs → crates/protocol/src/codec.rs
- `split_frame()` --calls--> `decode_varint()`  [INFERRED]
  crates/protocol/src/frame.rs → crates/protocol/src/codec.rs

## Import Cycles
- None detected.

## Communities (43 total, 24 thin omitted)

### Community 0 - "ARCHITECTURE.md — многопоточный Minecraft на Rust"
Cohesion: 0.11
Nodes (28): check_frame_len(), decode_frame(), decode_frame_len(), empty_payload_frame(), encode_frame(), frame_roundtrip(), length_field_over_three_bytes_is_rejected(), maximum_frame_length_is_accepted() (+20 more)

### Community 1 - "AGENTS.md — правила для ИИ-агента (opencode)"
Cohesion: 0.25
Nodes (7): AGENTS.md — правила для ИИ-агента (opencode), Главные правила, Если застрял, Как ориентироваться в проекте (graphify), Стиль кода, Тесты, Юридические границы (строго)

### Community 2 - "PROGRESS.md — прогресс проекта"
Cohesion: 0.17
Nodes (11): 10. Риски, 1. Цель, 2. Принципы, 3. Структура workspace, 4. Модель потоков (ядро проекта), 5. Данные и совместимость, 6. Клиент (после сервера), 7. Лаунчер (+3 more)

### Community 3 - "MineRust"
Cohesion: 0.18
Nodes (16): array_to_tag(), arrays_with_mixed_element_types_are_rejected(), collect_bytes(), collect_ints(), collect_longs(), converted_tags_roundtrip_through_the_network_codec(), dimension_type_style_object_converts(), float_or_double() (+8 more)

### Community 4 - "lib.rs"
Cohesion: 0.11
Nodes (31): Cursor, Cursor<'a>, decode_reports_consumed_bytes(), depth_limit_allows_512_and_rejects_513(), empty_lists_accept_any_element_type(), every_tag_type_roundtrips(), lengths_beyond_the_input_are_rejected(), NbtTag (+23 more)

### Community 5 - "CodecError"
Cohesion: 0.08
Nodes (25): CodecError, decode_varint(), encode_varint(), fixed_width_integers_are_big_endian(), ints_and_floats_are_big_endian(), nbt_roundtrips_through_the_packet_cursor(), PacketReader<'a>, PacketWriter (+17 more)

### Community 7 - "handshake.rs"
Cohesion: 0.24
Nodes (12): all_next_states_roundtrip(), Handshake, handshake_roundtrips_through_a_frame(), handshake_wire_layout_is_length_prefixed(), NextState, overlong_server_address_is_rejected(), Result, Self (+4 more)

### Community 8 - "config.rs"
Cohesion: 0.15
Nodes (21): AsRef, ConfigError, example_toml(), load_config(), parses_a_valid_config(), rejects_an_empty_client_jar_path(), rejects_empty_listen_address(), rejects_non_positive_max_players() (+13 more)

### Community 9 - ".to_json"
Cohesion: 0.14
Nodes (27): CodecError, configuration_state(), handle_connection(), login_state(), play_state(), Arc, GameData, StatusInfo (+19 more)

### Community 10 - "status.rs"
Cohesion: 0.19
Nodes (19): handshake(), read_until_close(), recv(), recv_frame(), Arc, Vec, send(), start_server() (+11 more)

### Community 11 - "NbtError"
Cohesion: 0.10
Nodes (43): BTreeMap, DataError, String, GameData, RegistryData, RegistryEntry, String, Vec (+35 more)

### Community 12 - "main.rs"
Cohesion: 0.52
Nodes (6): main(), parse_args(), Result, try_main(), ExitCode, PathBuf

### Community 14 - "login.rs"
Cohesion: 0.12
Nodes (23): disconnect_reason_carries_a_json_text_component(), DisconnectLogin, GameProfile, login_acknowledged_has_no_fields(), login_start_rejects_an_oversized_name(), login_start_roundtrips_name_and_uuid(), login_success_places_the_session_id_last(), login_success_roundtrips_a_profile_with_properties() (+15 more)

### Community 15 - "encode_frame"
Cohesion: 0.17
Nodes (19): jar_exactly(), jar_with(), String, Vec, write_jar(), a_registry_without_entries_is_an_error(), every_synchronized_registry_is_loaded(), load() (+11 more)

### Community 16 - "MineRust"
Cohesion: 0.22
Nodes (8): Building, Goals, Layout, Legal, License, MineRust, Running the server, Status

### Community 17 - "PROGRESS.md — прогресс проекта"
Cohesion: 0.29
Nodes (6): M0 — Каркас, M1 — Handshake и status, PROGRESS.md — прогресс проекта, Известные проблемы / блокеры, Что дальше, Что сделано

### Community 21 - "configuration.rs"
Cohesion: 0.13
Nodes (23): client_information_rejects_an_oversized_locale(), client_information_roundtrips_every_field(), ClientInformation, finish_ack_has_no_fields(), finish_configuration_has_no_fields(), FinishAck, FinishConfiguration, read_count() (+15 more)

### Community 22 - "Uuid"
Cohesion: 0.10
Nodes (29): PacketReader, Position, confirm_teleportation_roundtrips_the_echo(), ConfirmTeleportation, DeathLocation, game_event_carries_an_ubyte_and_a_float(), GameEvent, keep_alive_roundtrips_its_id() (+21 more)

### Community 23 - ".to_json"
Cohesion: 0.53
Nodes (4): json_escapes_special_characters(), json_exposes_version_players_and_motd(), String, StatusInfo

## Knowledge Gaps
- **28 isolated node(s):** `Status`, `Goals`, `Layout`, `Building`, `Running the server` (+23 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **24 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `CodecError` connect `CodecError` to `ARCHITECTURE.md — многопоточный Minecraft на Rust`, `handshake.rs`, `NbtError`, `login.rs`, `configuration.rs`, `Uuid`?**
  _High betweenness centrality (0.296) - this node is a cross-community bridge._
- **Why does `GameData` connect `NbtError` to `status.rs`, `encode_frame`?**
  _High betweenness centrality (0.106) - this node is a cross-community bridge._
- **Why does `DataError` connect `NbtError` to `config.rs`?**
  _High betweenness centrality (0.070) - this node is a cross-community bridge._
- **What connects `Status`, `Goals`, `Layout` to the rest of the system?**
  _28 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `ARCHITECTURE.md — многопоточный Minecraft на Rust` be split into smaller, more focused modules?**
  _Cohesion score 0.11149825783972125 - nodes in this community are weakly interconnected._
- **Should `lib.rs` be split into smaller, more focused modules?**
  _Cohesion score 0.11320754716981132 - nodes in this community are weakly interconnected._
- **Should `CodecError` be split into smaller, more focused modules?**
  _Cohesion score 0.0825136612021858 - nodes in this community are weakly interconnected._