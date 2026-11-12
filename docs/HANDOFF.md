# HANDOFF (state for the next session)

**M4.5 (сервер), Шаг 1 готов** — 12.11.2026, разобраны пакеты о другом игроке. База Шага 1: `e676a3c` (Шаг 0).
- Инструмент: `analyze.py entities-detail` (фильтр по id B=77, мобы отдельной строкой, `--list`); `python -m py_compile` зелёный.
- Факты (трейс С1/С2/С3): о id 77 — 9/174/814 пакетов (add, remove, move_entity_pos, move_entity_pos_rot, rotate_head, entity_position_sync, set_entity_data, update_attributes); мобы 19456/23139. `move_entity_rot` и `teleport_entity` о игроке — НЕ наблюдались. `add_entity`(77): uuid `b3b03a00-4714-337e-8103-52c60a6c836f`, type 159, len 48. Порядок входа: add → set_entity_data(11) → move_entity_pos → set_entity_data(14) → update_attributes(32) → [entity_position_sync] → move_entity_pos_rot + rotate_head. Граница видимости: макс. 160.2 (=R 10×16), одиночное мерцание 154.6/154.4.
- **Гипотеза Шага 3:** круг радиуса `view_distance×16`, без гистерезиса (мерцание — «не установлено»).
- **Требует решения владельца:** `update_attributes` и `entity_position_sync` о игроке есть в трейсе, но их нет в списке кодеков Шага 2 → точное равенство списка в Шаге 4в недостижимо без них (молча не расширяю).
- Доки: `docs/ARCH-M4.md` §6 (новый подраздел «Шаг 1»); разборы `target/bench/m45-detail-c{1,2,3}[-list].txt`.
- **Дальше — Шаг 2:** кодеки в `crates/protocol` (только добавление) с тестами «те же байты из трейса»; решить/пометить `teleport_entity`/`update_attributes`/`entity_position_sync`. Затем Шаги 3–5 (видимость в планировщике + замер с порогом steady p99 P=200 > 5 мс = СТОП; тесты а–д; доклад + LIVE-CHECK).
- Изменяемое на Шагах 1–5: `crates/net`, конфиг `crates/server`, `crates/protocol` (add-only), `crates/bench`, бот/тесты `crates/parity` (новые), `tools/`, `docs/`. `server.toml` владельца не трогать. Замеры — только при простое машины. Трейсы: `C:\MineRust-data\traces\`.
