# HANDOFF (state for the next session)

**M4.5 (сервер), Шаги 7.1–7.5 выполнены** — 13.11.2026. База: 7.1 `e251b39`, 7.2 `c66a650`/`0830200`, 7.3a `397dbb2`, 7.3 `34e607b`.
- **Причина живого «наблюдателя» (7.1)**: `entity_packets.rs` слал константы из spectator-трейса C1 — `PLAYER_INFO_GAME_MODE = 3` и entity flags `0x20` (невидимость) во втором `set_entity_data`. Login слал `game_mode: 0`; нормализация в (в) скрывала первое.
- **7.2 (ванильные C5/C6, `two_bot_gamemode.rs`)**: от режима зависят `player_info_update` offset 30 (`03`/`00`/`01`) и состав второго `set_entity_data` (`0x20` — только spectator, 14 Б; в C5/C6 11 Б, как первый); 11 Б базового снапшота одинаковы везде. Третья зависимость найдена в 7.3: первый байт нулевого `move_entity_pos` — `02` в C1, `03` в C5/C6.
- **7.3 (режим как параметр)**: `GameConfig::default_game_mode` (default 0, `0..=3`, иначе ошибка старта), `PlayerJoin/Player/EntitySpawn/EntityMove.game_mode`, `metadata_second`, заголовок `ZeroDelta` по режиму. В `server.toml`/`config.rs` **не добавляли** (решение владельца). Тест (в) без нормализации режима: C1 — 1 дифф (iii), C5/C6 — по 6 строк (i + iii + сдвиг на пакет), байтовых диффов по режиму нет; C2/C4 — прежние (i)/(ii)/(iv). net 117 ok, gate зелёный.
- **7.4 (опоздания тика)**: тик стартует **после** прогрева (`build_world` → `warm_world` → `bind` → `serve_with`); замер 20 с без игроков — 400 тиков, `late_ticks=0`, `late_max_ms=0.000`. Живые 7–9 с / 103 мс **не установлено** — нужен лог с `uptime_ms` и меткой входа.
- **Открыто**: остатки (в) (i)–(iv) — за владельцем; долг «три `player_info_update` на входе» (self-записи) не закрыт; spectator (3) поддержан только байтами о других игроках (нет `player_abilities`/полёта).
- **Вне M4.5 (M5+)**: экипировка, скины, анимации, чат, атаки/предметы, мобы. `server.toml` не трогать.
- Graph: `graphify update .` после правок кода.

## M4.5 (сервер), Шаг 8 — код написан, ждём трейсы S1–S4 (14.11.2026)

Код по принятому плану Шага 8: коммиты (a) `e0ccda6` player_input/punch +
состояние игрока, (b) `28fe8bd` фазовая диагностика тика, (c) non_blocking
stdout writer, (d) этот документ + запись трейсов. Gate зелёный на каждом
шаге (fmt, clippy -D warnings, test --workspace).

**Что сделано:** serverbound `player_input` (0x2B)/`player_command` (0x2A)/
`punch` (0x2E) декодируются с тестами по живому трейсу; состояние игрока
несёт sneak/sprint (из `player_input`, биты 0x20/0x40 — гипотеза, помечена
«SOURCE: wiki, unverified»), счётчик swings и `head_yaw` (следует за телом).
`MoveSlot` хранит движение/ввод/взмахи независимо (новых `Mutex` нет).
Warn «tick phases» при late > 50 мс: `late_ms = finished − deadline` (общее
опоздалость), `wake_late_ms = woke − deadline` (только пробуждение) —
большой wake при малых фазах = стопор снаружи (ОС/консоль/антивирус).
`tracing-appender`, `lossy(false)`, guard в `main` (backpressure при полном
буфере, flush до 1 с при выходе). Запись трейсов автоматизирована:
`tools/record-step8.ps1 -Scenario S1|S2|S3|S4` (+`-DryRun`), разбор —
`tools/trace-analyze/m45-step80-scenarios.py`.

**Что НЕ сделано (ждёт трейсов S1–S4):** кодировщики `add_entity`,
`set_entity_data`, `animate` для наблюдателей; байт-правила позы (B),
взмаха (C), плавания (D) и поворот при входе (A). **Клиентбайты не
придумываем.** Живая проверка после Шага 8 показала те же дефекты A–D, что и
до кодировщиков — это ожидаемое состояние (поза/взмах/плавание/поворот ещё
не кодируются для наблюдателей).

**Дальше (Шаг 9):** владелец записывает трейсы S1–S4
(`docs/RUNBOOK-STEP8-TRACES.md`, `tools/record-step8.ps1`), приносит
`<сценарий>-A/B.jsonl` + `summary.txt`; по ним пишем кодировщики наблюдателю
и байт-правила. Тест (в) (`parity/tests/entity_stream_parity.rs`) остаётся
красным диагностическим до кодировщиков.
