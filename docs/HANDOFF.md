# HANDOFF (state for the next session)

**M4.5 (сервер), Шаг 3 готов** — 12.11.2026, видимость и entity-поток. База: `6c1a850`.
- Новый `crates/net/src/game/entities.rs`: `visibility_pass` (горизонтальный круг `view_distance*16`, без гистерезиса, симметрия через `min(r_a,r_b)`, себя не видит) + типы `StreamEntities/EntitySpawn/EntityRemove/EntityMove/EntityMotion`; тесты рядом (`entities_tests.rs`).
- `Player.visible`/`last_broadcast`, `TickOutcome.entities`, транскрипт: новый `kind: entity` (сорт 3), kick переехал на сорт 4; replay-journal не менялся. `ServerMessage::StreamEntities` + `deliver_entities` после `deliver_streams`.
- Новый `crates/net/src/connection/play/entities.rs`: порядок спавна как в C1 (player_info_update → add_entity → set_entity_data(11) → move_entity_pos(0x00) → set_entity_data(14) → update_attributes); движение Position/Rotation/Sync (смена позиции+поворот → pos_rot+rotate_head); удаление — player_info_remove только при уходе из мира, затем remove_entities.
- Движение идёт только зрителям из `old ∩ target`; спавн сбрасывает общий baseline; скачок >±8 блоков → entity_position_sync.
- `PLAYER_INFO_GAME_MODE = 3` (в трейсе C1 записано 3, не 0 — доверяем трейсу).
- Гейт: fmt / clippy --all-targets / test — зелёный (61 ok). Дальше — Шаг 3.5 (замер, две геометрии).
- Изменяемое: `crates/net`, конфиг `crates/server`, `crates/protocol` (add-only), `crates/bench`, `crates/parity` (новые), `tools/`, `docs/`. `server.toml` не трогать. Замеры — только при простое.
- Graph: обновлён (`graphify update .`).