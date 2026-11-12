# HANDOFF (state for the next session)

**M4.5 (сервер), Шаг 2 готов** — 12.11.2026, кодеки entity-потока. База: `c6c44ba` (C4).
- Новые модули `crates/protocol/src/packets/{entity,movement,entity_meta}.rs` (+`packet_impl`, `test_support`): add_entity, remove_entities, player_info_remove, move_entity_pos, move_entity_pos_rot, move_entity_rot, rotate_head, set_entity_data, update_attributes, entity_position_sync; тесты воспроизводят байты трейса C1–C4.
- `player_info_update`: добавлен только тест на B (game mode 3, latency 0); кодек не менялся. `teleport_entity` не добавляли.
- Заголовки move-пакетов, хвост add_entity и поля entity_position_sync — «не установлено» и вынесены явными полями.
- Гейт: fmt / clippy --all-targets / test — зелёный (61 ok). `used_packet_keys` расширен 10 ключами (в отчёте владельца все id есть).
- Дальше — Шаг 3: видимость в планировщике (снимок, BTreeSet, StreamEntities, порядок по §6) + замер 3.5 (две геометрии).
- Изменяемое: `crates/net`, конфиг `crates/server`, `crates/protocol` (add-only), `crates/bench`, `crates/parity` (новые), `tools/`, `docs/`. `server.toml` не трогать. Замеры — только при простое.
- Graph: обновлён (`graphify update .`).