# HANDOFF (state for the next session)

**Шаг 16: 16.0a–d и 16a.1 закрыты; 16a.2 готов наполовину и ждёт владельца.**
16.11.2026. Пуш прошёл: `main` в синхроне с `origin/main` (последний коммит
16a.1 — `fb1d771`).

**Блокер 16a.2 (одно действие владельца):** нужен
`E:\MineRust-data\reports\hardness.json` — запустить подключённым тест-клиентом
команду `hardness` (мод-джар уже пересобран с ней):

```powershell
. E:\projects\MineRust\tools\testclient\scenario-lib.ps1
Invoke-TestCmd -Port 25571 -Json '{"id":1,"cmd":"hardness","args":{"path":"E:/MineRust-data/reports/hardness.json"}}'
```

Новый ванильный прогон, судя по данным, **не нужен**: в трейсах 14.11.2026 уже
есть 4 блока разной твёрдости (листва, dirt, grass, dark_oak_log). После файла
16a.2 закрывается сверкой `T ≈ 30·hardness` (детали —
`docs/BLOCK-BREAKING.md` §1.7).

- **16.0a — `collision.json`/`reports/`/`E:\MineRust-data` не в репозитории;
  сервер без `collision_report` работает.** `git ls-files` даёт только
  `crates/data/src/collision.rs` (загрузчик). Отсутствие настройки выключает
  правило ползания (`free_headroom` = `None`) и пишет `warn`; тест
  `a_server_without_the_collision_report_disables_the_crawl_rule_and_warns`.
- **16.0b — красный диагностический прогон (в).** `entity_stream_parity
  --ignored`: **1 failed**, 286 расхождений (C1 1, C2 173, C4 100, C5/C6 по 6).
  Сравнение идёт **по номеру пакета**, поэтому корень один: у нас нет
  `entity_position_sync` о самом игроке и другой темп `rotate_head`/
  `move_entity_*` (причины Шага 4). Тест **не менялся**. Отчёт
  `C:\MineRust-data\traces\entity-stream-parity.md`.
- **16.0c — готовые dev_geometry-сцены.** `tools/testclient/geometry/`:
  `pond.txt`, `crawl.txt`, `waterlogged.txt`, `README.md` (строки `server.toml`,
  команда `observe`, таблицы ожидаемого поведения B). По умолчанию **не
  включаются**; тест `shipped_dev_geometry_files_parse`.
- **16.0d — сводка долгов механик** (таблица ниже).
- **16a.1 — экспорт твёрдости + серверная настройка `hardness_report`.**
  Команда `hardness` тест-клиента пишет `reports/hardness.json`
  (`BlockState.getDestroySpeed` + `Player.getDestroySpeed` пустой рукой);
  загрузчик `crates/data::BlockHardness`; без настройки `GameConfig.hardness =
  None` + `warn`, с битым путём — отказ старта.
- **16a.2 — модель стадий/времени выведена из трейсов.** `stage` =
  `floor(progress·10)` без клиппинга (0..13 в трейсах), `0xFF` — сброс (и при
  прерывании), шлётся только при смене стадии; прогресс +`speed/hardness/30` за
  тик; **сервер не ломает блок сам** — блок убирает клиентский
  `player_action 0x03`. Длительности: листва ≈6 тиков, dirt ≈15, grass ≈16,
  dark_oak_log ≈60. Таблица — `docs/BLOCK-BREAKING.md` §1.7, проверка гипотез —
  §5 п.7.
- **Дальше:** 16a.2 закрыть по `hardness.json` (см. блокер выше), затем 16a.3 —
  серверный таймер + рассылка стадий наблюдателям, 16a.4 — тесты/оракул/live,
  16a.5 — не делать дропы/инструменты/звук (кроме уже сделанного `level_event`
  2001). План — `docs/BLOCK-BREAKING.md`; реестр — `docs/MECHANICS.md`.

## Долги механик плавания/ползания (16.0d)

Не реализованы (Шаг 15 п.6). В `docs/MECHANICS.md` строка 12 перечисляет их
по частям; здесь одна точка входа.

| Долг | Суть | Данные есть? |
|---|---|---|
| Симуляция жидкости | нет перетекания/растекания воды (в S10 `G4-wl-slab-bottom` ваниль разнесла воду, у нас клетка `air`) | частично (S10) |
| Модель коллизий | нет столкновений: поза понижается по месту над **текущей** позицией, не по попытке входа; `G4-wl-fence` при sneak | нет |
| Воздух (`air supply`) | index 1 не шлётся (ваниль 299→201), `oxygen_bonus` (key 28) не шлётся | есть (S8, 13c) |
| Утопление | урон/регенерация от воздуха | нет |
| Звук всплеска | звуков входа/выхода из воды нет (в S8 ваниль тоже не слала) | не подтверждён |
| Подводность глаз | старт плавания по глазам под водой | нет (r3 не различает) |

Подробности и ссылки на разделы — `docs/SWIMMING.md` §2.1/§2.2/§5.

## История (M4.5)

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
