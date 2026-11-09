# M4.0 (а) — раунд 2: полный контекст для новой сессии

Задача владельца (2026-11-09): правки тестового инструмента walk-трейса,
три прогона П1-П3 на свежих копиях v2, замеры скорости входа vanilla vs
наш сервер, ОДИН финальный отчёт в конце. Это тестовый инструмент, НЕ M4.1;
код сервера не менять.

Правила: todo-лист с одним `in_progress`; после каждого пункта — коммит,
HANDOFF (≤10 строк), push; большие выводы в файлы; при потере нити писать
«нужна новая сессия»; ничего не выдумывать (не нашёл — сказать); финальный
отчёт ОДИН (короче прошлого, без полного списка типов пакетов).
Трейсы в `C:\MineRust-data\traces\`, в репозиторий только сводка.

## Что уже сделано (все коммиты запушены)

| Коммит | Что |
|---|---|
| 775e4f9 | fix: baseline мира — SHA шаблона (раньше хэш копии до старта; на Windows сервер держит файлы копии открытыми, before-хэш невозможен) |
| 1d31f17 | feat: метка монотонного времени на сокете для КАЖДОГО пакета: `Client::stamps` (Instant от `opened` до connect), `record()` пишет traces+stamps синхронно; JSONL `t_ms`, `Batch.t_ms`, `CacheMove.t_ms` берут метку пакета, не время записи строки |
| 2d826e2 | feat: `client_tick_end` (id 13) каждые 50 мс по абсолютной сетке, якорь — первый teleport ack; `player_loaded` через 1001 мс после ack (живой клиент: seq 52→91 = +1001.28 мс); recv просыпается по ближайшему из дедлайнов движение/тик/loaded; `Timers` |
| fcce22c | fix: `save-all flush` + `stop` только после того, как бот завершился сам: walk-future ставит `quit` после возврата `run_walk`, серверное будущее ждёт флаг (опрос 50 мс), затем стоп |
| c532728 | feat: режим подтверждается по трейсу: `login.game_mode` (полный разбор `LoginPlay`, id 50) и первый байт `player_abilities` (id 65); несовпадение/неразбор → `Stop::Mode`; факты в `RunStats.game_mode/abilities_flags` и в отчёт |
| b4d151c | test: execute-пробы из `walk_trace.rs` удалены, вместо них assert `game_mode == 3 && abilities_flags == 0x07` → нарушение → abort последовательности, факты в отчёте |
| 3ec8b97 | test: мир через `chunk_status.py compare` + зона x -7..19 / z -15..11 (`tests/common/world_criterion.rs`); region SHA/`entities`/`poi` — только информация, шаблон сверяется по SHA |
| b74ac44 | fix: `ms()` возвращал СЕКУНДЫ под видом мс (`(secs*1000).round()/1000`): 10.8 с бота читались как `t_ms=10.803`. Исправлено на `(secs*1e6).round()/1000`; `elapsed_ms` тоже через `ms()`; два юнит-теста |

П.1-5 ЧАСТИ I — готовы полностью: walk.rs собирает факты режима и
останавливается сам (`Stop::Mode`) вместо execute-проб (b4d151c),
мировой критерий — `chunk_status.py compare` + зона (3ec8b97) вместо
abort по region SHA. Дополнительно найден и исправлен дефект часов
трейса (b74ac44, см. факты ниже) — прогоны П1-П3 надо повторить с
исправленными временами.

## Ключевые факты (проверенные, не выдумывать заново)

- Идентификаторы serverbound play: `accept_teleportation` 0,
  `client_tick_end` 13, `player_loaded` 44 (`0x2C`),
  `move_player_pos` 30, `move_player_pos_rot` 31,
  `client_information` 14, `chunk_batch_received` 11.
  Clientbound: `login` 50, `player_abilities` 65.
- Pace float `chunk_batch_received` = hex `40957d29` (≈4.6715).
- Живой клиент (live-session.json): ack seq 52 t=84114.5607; первый
  tick_end seq 53 t=84143.4622; `player_loaded` seq 91 t=85115.8432
  (= ack + 1001.28 мс); порядок seq 51-91 выгружен в
  `C:\MineRust-data\traces\live-seq-51-91.txt`.
- П1 прошлого раунда: 173 чанка (при view 10 ожидается 441), 42 порции,
  8 926 097 B, spawn (96.5, 145, -31.5), 0 forget_level_chunk,
  1 set_chunk_cache_center (6, -2); обрыв 10053 через ~5 мс play;
  `player_abilities` payload `07 3d4ccccd 3dcccccd`; лог сервера
  `Default game type: SPECTATOR`. Времена в трейсе того прогона считать
  нельзя (см. ниже про `ms()`).
- ИСПРАВЛЕНИЕ (b74ac44): «сломанные времена» не были устранены в 1d31f17
  полностью — `ms()` делил уже миллисекунды на 1000 и возвращал СЕКУНДЫ.
  Заметки на сокете корректны (`Instant` от connect), перепутана была
  только единица. Поэтому в трейсах ДО b74ac44 все `t_ms` меньше в 1000 раз
  (напр. 10.8 с бота → 10.803). Архив таких прогонов: `traces\bugtimes-*`.
- Прогон 2026-11-09 ДО фикса часов (архив `bugtimes-*`, времена ×1000):
  П1 — stop=Completed, teleports=1, режим ok, 346 чанков, 85 закрытых
  порций (86 start), 1 cache_center (6,-2), новых чанков 0, критерий мира
  прошёл (template 2209 / copy 2209); П2 — 69 чанков, 16 порций, кик
  ванилью: лог `ParityBot lost connection: Invalid move player packet
  received` после 34 отправленных `move_player_pos` (stop Closed os error
  10054), П3 не запускался (правило остановки).
- Прогон 2026-11-09 ПОСЛЕ фикса часов (`walk-run-2-2026-11-09.log`):
  П1 — elapsed 10004 мс, 428 `level_chunk_with_light` (20844384 B),
  first 1549.394 / last 10747.368 мс, 106 finished / 107 start порций,
  последняя порция на 10707.008 мс — сервер ещё слал при выходе бота;
  client_tick_end gaps median 50.900 мс (до фикса читалось 0.051);
  intention 3.613 → login 1425.649 (1422.0 мс) → первый чанк 1549.394
  (123.7 мс после login); player_position seq 54 = 1427.962, ack
  seq 55 = 1428.169, player_loaded seq 563 = 2440.612. П2 — intention
  1.385 → login 1155.326 (1153.9 мс) → первый чанк 1263.229 (107.9 мс);
  59 чанков, 13 порций; 30 move с 1186.132 по 2678.533; кик ванили
  `Invalid move player packet received`. П3 не запускался.
- Ответ 173/441: 173 — не предел сервера. В прошлом раунде P1 оборвался
  рано (часы были ×1000, «5 мс play» = ~5 с), поэтому успело прийти 173
  чанка. С честными 10 с приходит 428, и поток продолжается — 441
  достигается при ожидании дольше 10 с (цель ЧАСТИ III: ждать 441 чанк
  или 30 с).
- Зона движения (assert ДО отправки): центральный чанк x 3..9, z -5..1;
  блоки x 50..157, z -78..29 (2 блока запаса).
- Зона критерия мира: новые чанки допустимы только в x -7..19, z -15..11;
  Status-изменения внутри зоны перечислить, не останавливаться.
- Копия v2: `world-template`; `Vanilla::start` каждый запуск удаляет
  `vanilla\world`, копирует шаблон, стирает старый `logs/latest.log` —
  факты из лога копировать до следующего прогона.
- Сервер (наша копия конфигурации): view-distance 10, spectator,
  `network-compression-threshold=-1` (записать значение в отчёте).
- `chunk_status.py` на устаревшей копии П1 (2026-11-09, smoke-проверка):
  template 2209 чанков, копия 2209, `only_in_B` и `status_diffs` пусты,
  `heightmap_diffs=5`, `light_presence_diffs=11`; прошлый раунд остановил П1
  старым критерием (4 region/*.mca поменялись) вместе с обрывом os error
  10053 — region SHA теперь только информация.
- Порции/интервалы из трейсов П1-П2 (факт, пересчитан `trace_facts.py`/
  чтением JSONL): П1 — 106 закрытых порций, размеры min/med/max = 1/4/9
  (среднее 4.00), интервалы между `chunk_batch_finished` min/med/max =
  30.3/78.5/216.1 мс; П2 — 13 порций, 4/4/9 (среднее 4.38),
  интервалы 65.6/86.1/188.4 мс. `set_chunk_cache_center` ровно один,
  в обоих: П1 на 1428.808 мс, П2 на 1158.363 мс, центр (6,-2)
  (payload 6 байт, две varint).
- ЧАСТЬ III (замеры 2026-11-09, `preload-report.md`, все 12 прогонов
  `Completed`, по 441 чанк, 108 порций у vanilla / 87 у нас):
  мин/медиана/макс intention→441-й чанк: vanilla cold 8913.0 / 10019.2 /
  12534.6 мс; vanilla warm 6295.8 / 6534.2 / 6672.7; наши cold
  12047.1 / 13033.7 / 13433.6; наши warm 13865.4 / 13915.8 / 14409.9.
  В серии «наши» заложены ~3000 мс ожидания бота (`PROBE_WAIT`). Пайтлоад
  чанков: vanilla ≈ 21.39 MB, наш ≈ 39.37 MB (одинаково во всех
  повторах). int→login: vanilla cold 1169.7/1260.4/1765.4, vanilla warm
  249.1/260.8/273.9, наши cold 3007.3/3012.5/3015.4, наши warm
  3004.2/3006.8/3007.3 (серии наших без поправки бота: см. PROBE_WAIT).
  Режим (записан, не судился): vanilla `game_mode 3 / flags 0x07`,
  наши — `game_mode 0`, `player_abilities` не приходит (факт прогона).
  Сжатие: vanilla `network-compression-threshold=-1` (выключено), наш
  сервер `SetCompression` не шлёт (`crates/net` — их нет), т.е. порога нет.
  В ходе серий «наши» в вывод некрасиво вплелись строки с кракозябрами
  (`--nocapture`, кодировка OEM→UTF-8, источник строк не выяснен);
  в отчёт и в трейсы они не попадают.

## Что осталось (план новой сессии)

1. [ГОТОВО] Добить п.4: execute-пробы из `crates/parity/tests/walk_trace.rs`
   удалены (константы `CHECK_*`/`GM_*`, enum `GmStatus`, `outcome.gm`,
   `probe_err`, чтение лога, секция «Gamemode probes»); вместо них проверка
   `stats.game_mode == Some(3) && stats.abilities_flags == Some(0x07)` —
   не сходится → нарушение → abort последовательности, факты в отчёте
   (mode from the trace) и в walk-summary.md.
2. [ГОТОВО] П.5 ЧАСТИ I: критерий мира через `target/parity/chunk_status.py`
   `compare <template> <world>` (модуль `tests/common/world_criterion.rs`):
   `only_in_B` парсится, стоп только при новых чанках вне зоны x -7..19,
   z -15..11; `status_diffs` внутри/снаружи зоны перечисляются без стопа;
   каталог шаблона сверяется по SHA (до/после всех прогонов), `entities/`
   и `poi/` — отдельными списками; region SHA убран из критерия, остаётся
   информацией. Отказ скрипта/нулевой скан шаблона = нарушение.
3. [ГОТОВО] Прогоны П1-П3 после b74ac44 (`walk-run-2-2026-11-09.log`).
   П1 (стоять 10 с) чист: stop=Completed, elapsed 10004 мс, teleports=1,
   режим ok (game_mode 3, flags 0x07), 428 `level_chunk_with_light`,
   106 закрытых порций / 107 start, 1 cache_center (6,-2), критерий мира
   прошёл (template 2209 / copy 2209, only_in_B 0). П2 (прямая +x) снова
   убит ванилью: лог `Invalid move player packet received`, обрыв 10054
   после 30 `move_player_pos`; 59 чанков, 13 порций. П3 по правилу
   остановки не запускался. Это ожидаемое поведение (ничего не тюнили).
4. [ГОТОВО] ЧАСТЬ III: 173/441 разобран (см. «Ключевые факты»), замеры
   проведены — `C:\MineRust-data\traces\preload-report.md` + 14 JSONL
   (12 измеренных + 2 warm-up). Оркестратор `tests/preload_measure.rs`
   (6b91786), метрики `src/preload.rs` (d53bf03). Все 12 прогонов
   `Completed` по 441 чанк, бюджет 30 с никто не исчерпал.
   **Важно по трейсу**: у нашего сервера бот сам ждёт `PROBE_WAIT` 3 с
   (`steps.rs`: `respond` не выходит без `select_known_packs`, которого наш
   сервер не шлёт) — первый `finish_configuration` боту приходит на
   20.7 мс, а подтверждение бот шлёт на 3003.9 мс. Все времена серий «В»
   содержат эти ~3000 мс ожидания бота; поправка на них — не вывод
   владельца, а арифметика самих трейсов (см. финальный отчёт).
5. [ГОТОВО — отчёт выдан в этой сессии] Финальный отчёт (структура владельца): по П1-П3 — время жизни бота;
   счётчики ключевых типов (`set_chunk_cache_center`, `forget_level_chunk`,
   `player_position`, `chunk_batch_start/finished/received`,
   `level_chunk_with_light`, `client_tick_end`, `move_player_*`);
   последовательность cache-move с монотонным временем и позицией бота;
   размеры/интервалы порций; результат п.5 (режим) и п.6 (мир); таблица
   ЧАСТИ III min/median/max; ответ 173/441; раздельно «решено по трейсу»
   и «гипотеза»; показать запись seq 53 и порядок seq 51-91.

## Технические тонкости

- `walk.rs`: `t_ms` теперь от начала соединения (не от play); время
  жизни бота — `RunStats.elapsed_ms` от старта play-цикла.
- `recv_packet(wake)` — `wake = min(дедлайн движения, timers.tick,
  timers.loaded)`; иначе тики задерживались бы до сетки движения.
- `process_recorded` обрабатывает ВСЕ записанные пакеты, включая джойн,
  при первом проходе play-цикла; индексы `stamps` == `trace.packets`.
- Windows: перед каждым прогоном копировать нужные строки из
  `logs/latest.log` (следующий `Vanilla::start` его удалит).
- Проверки перед коммитом: `cargo fmt`,
  `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`
  (все три обязательны и должны быть зелёными).
- Прогоны: `cargo test -p parity --test walk_trace -- --ignored --nocapture`
  (п.4-хвост и п.5 готовы — можно запускать).
- После правок кода: `graphify update .` (AST-only).
- `docs/PROGRESS.md` обновить после этапа; коммиты Conventional Commits.

## Файлы

- `crates/parity/src/client.rs` — `stamps`/`arrivals`, метки на сокете.
- `crates/parity/src/walk.rs` — бот: таймеры, тики, player_loaded,
  cache-move/порции, факты режима, Stop.
- `crates/parity/tests/walk_trace.rs` — оркестратор 3 прогонов, отчёт,
  diff мира, факт режима ЧАСТЬ I п.4; п.5 через world_criterion.
- `crates/parity/tests/common/world_criterion.rs` — `chunk_status.py
  compare`, зона, парсинг + юнит-тесты.
- `crates/protocol/src/packets/play.rs` — `LoginPlay` (game_mode).
- `target/parity/chunk_status.py` — критерий мира (`scan`/`compare`).
- `target/parity/live-session.json` — живой трейс.
- `C:\MineRust-data\reports\packets.json` — идентификаторы пакетов.
- `C:\MineRust-data\traces\` — трейсы, отчёты, `live-seq-51-91.txt`.