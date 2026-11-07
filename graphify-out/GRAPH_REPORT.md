# Graph Report - MineRust  (2026-11-07)

## Corpus Check
- 35 files · ~28,714 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 611 nodes · 1473 edges · 23 communities (19 shown, 4 thin omitted)
- Extraction: 97% EXTRACTED · 3% INFERRED · 0% AMBIGUOUS · INFERRED: 39 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `842fb1e8`
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

## God Nodes (most connected - your core abstractions)
1. `CodecError` - 28 edges
2. `DataError` - 21 edges
3. `GameData` - 21 edges
4. `PacketReader<'a>` - 18 edges
5. `PacketWriter` - 18 edges
6. `load()` - 16 edges
7. `read_frame()` - 16 edges
8. `NbtError` - 15 edges
9. `handle_connection()` - 14 edges
10. `ConnectionError` - 14 edges

## Surprising Connections (you probably didn't know these)
- `a_registry_without_entries_is_an_error()` --calls--> `jar_exactly()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/fixtures.rs
- `compiled_packet_ids_match_the_real_report()` --calls--> `load_packets_report()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/packets.rs
- `report_ids_override_the_compiled_defaults()` --calls--> `packets_from_json()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/packets.rs
- `report_with_invalid_json_names_the_file()` --calls--> `packets_from_json()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/packets.rs
- `report_without_a_needed_packet_is_rejected()` --calls--> `packets_from_json()`  [INFERRED]
  crates/data/src/tests.rs → crates/data/src/packets.rs

## Import Cycles
- None detected.

## Communities (23 total, 4 thin omitted)

### Community 0 - "ARCHITECTURE.md — многопоточный Minecraft на Rust"
Cohesion: 0.19
Nodes (14): ping_and_pong_roundtrip_payloads(), PingRequest, PongResponse, CodecError, PacketReader, Result, Self, String (+6 more)

### Community 1 - "AGENTS.md — правила для ИИ-агента (opencode)"
Cohesion: 0.25
Nodes (7): AGENTS.md — правила для ИИ-агента (opencode), Главные правила, Если застрял, Как ориентироваться в проекте (graphify), Стиль кода, Тесты, Юридические границы (строго)

### Community 2 - "PROGRESS.md — прогресс проекта"
Cohesion: 0.07
Nodes (26): 10. Риски, 1. Цель, 2. Принципы, 3. Структура workspace, 4. Модель потоков (ядро проекта), 5. Данные и совместимость, 6. Клиент (после сервера), 7. Лаунчер (+18 more)

### Community 3 - "MineRust"
Cohesion: 0.10
Nodes (34): array_to_tag(), arrays_with_mixed_element_types_are_rejected(), collect_bytes(), collect_ints(), collect_longs(), converted_tags_roundtrip_through_the_network_codec(), dimension_type_style_object_converts(), float_or_double() (+26 more)

### Community 4 - "lib.rs"
Cohesion: 0.20
Nodes (18): Cursor, Cursor<'a>, decode_reports_consumed_bytes(), depth_limit_allows_512_and_rejects_513(), empty_lists_accept_any_element_type(), every_tag_type_roundtrips(), lengths_beyond_the_input_are_rejected(), NbtTag (+10 more)

### Community 5 - "CodecError"
Cohesion: 0.08
Nodes (27): CodecError, decode_varint(), encode_varint(), fixed_width_integers_are_big_endian(), ints_and_floats_are_big_endian(), nbt_roundtrips_through_the_packet_cursor(), PacketReader<'a>, PacketWriter (+19 more)

### Community 7 - "handshake.rs"
Cohesion: 0.24
Nodes (14): all_next_states_roundtrip(), Handshake, handshake_roundtrips_through_a_frame(), handshake_wire_layout_is_length_prefixed(), NextState, overlong_server_address_is_rejected(), CodecError, PacketReader (+6 more)

### Community 8 - "config.rs"
Cohesion: 0.16
Nodes (20): ConfigError, example_toml(), load_config(), parses_a_valid_config(), rejects_an_empty_client_jar_path(), rejects_empty_listen_address(), rejects_non_positive_max_players(), rejects_unknown_fields() (+12 more)

### Community 9 - ".to_json"
Cohesion: 0.09
Nodes (41): BTreeMap, GameData, RegistryData, RegistryEntry, NbtTag, Option, String, Vec (+33 more)

### Community 10 - "status.rs"
Cohesion: 0.20
Nodes (18): handshake(), read_until_close(), recv(), recv_frame(), Arc, TcpStream, Vec, send() (+10 more)

### Community 11 - "NbtError"
Cohesion: 0.14
Nodes (35): DataError, String, collect_tags(), load_from_jar(), load_from_zip(), load_registry(), read_json(), read_text() (+27 more)

### Community 12 - "main.rs"
Cohesion: 0.52
Nodes (6): main(), parse_args(), Result, try_main(), ExitCode, PathBuf

### Community 15 - "encode_frame"
Cohesion: 0.07
Nodes (39): AsRef, jar_exactly(), jar_with(), Vec, write_jar(), load_packets_report(), PacketIds, packets_from_json() (+31 more)

### Community 16 - "login.rs"
Cohesion: 0.13
Nodes (25): disconnect_reason_carries_a_json_text_component(), DisconnectLogin, GameProfile, login_acknowledged_has_no_fields(), login_start_rejects_an_oversized_name(), login_start_roundtrips_name_and_uuid(), login_success_places_the_session_id_last(), login_success_roundtrips_a_profile_with_properties() (+17 more)

### Community 17 - "PacketKey"
Cohesion: 0.14
Nodes (14): packets_report_json(), String, describe_shows_state_direction_and_name(), Direction, every_state_and_direction_has_a_distinct_index(), keys_of_the_same_packet_in_two_directions_differ(), PacketKey, ProtocolState (+6 more)

### Community 18 - "frame.rs"
Cohesion: 0.25
Nodes (17): PacketReader, check_frame_len(), decode_frame(), decode_frame_len(), empty_payload_frame(), encode_frame(), frame_roundtrip(), length_field_over_three_bytes_is_rejected() (+9 more)

### Community 21 - "configuration.rs"
Cohesion: 0.14
Nodes (26): client_information_rejects_an_oversized_locale(), client_information_roundtrips_every_field(), ClientInformation, finish_ack_has_no_fields(), finish_configuration_has_no_fields(), FinishAck, FinishConfiguration, read_count() (+18 more)

### Community 22 - "Uuid"
Cohesion: 0.11
Nodes (30): confirm_teleportation_roundtrips_the_echo(), ConfirmTeleportation, DeathLocation, game_event_carries_an_ubyte_and_a_float(), GameEvent, keep_alive_roundtrips_its_id(), KeepAlive, login_play_roundtrips_a_death_location() (+22 more)

## Knowledge Gaps
- **29 isolated node(s):** `Cursor`, `Главные правила`, `Юридические границы (строго)`, `Стиль кода`, `Как ориентироваться в проекте (graphify)` (+24 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **4 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `GameData` connect `.to_json` to `config.rs`, `status.rs`, `NbtError`, `encode_frame`?**
  _High betweenness centrality (0.215) - this node is a cross-community bridge._
- **Why does `CodecError` connect `CodecError` to `.to_json`, `frame.rs`, `MineRust`?**
  _High betweenness centrality (0.205) - this node is a cross-community bridge._
- **Why does `DataError` connect `NbtError` to `lib.rs`, `encode_frame`?**
  _High betweenness centrality (0.171) - this node is a cross-community bridge._
- **What connects `Cursor`, `Главные правила`, `Юридические границы (строго)` to the rest of the system?**
  _29 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `PROGRESS.md — прогресс проекта` be split into smaller, more focused modules?**
  _Cohesion score 0.06896551724137931 - nodes in this community are weakly interconnected._
- **Should `MineRust` be split into smaller, more focused modules?**
  _Cohesion score 0.09551020408163265 - nodes in this community are weakly interconnected._
- **Should `CodecError` be split into smaller, more focused modules?**
  _Cohesion score 0.07987711213517665 - nodes in this community are weakly interconnected._