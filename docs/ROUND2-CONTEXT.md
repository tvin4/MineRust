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

П.1-4 ЧАСТИ I — готовы полностью: walk.rs собирает факты и останавливается
сам (`Stop::Mode`); execute-пробы из `walk_trace.rs` удалены, вместо них
проверка `game_mode == 3 && abilities_flags == 0x07` → нарушение → abort
последовательности, факты в отчёте (см. «Осталось», п.1 — сделано).

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
  `Default game type: SPECTATOR`. Причина сломанных времён в трейсе
  (все 0.06-5.08 мс) устранена в 1d31f17.
- Зона движения (assert ДО отправки): центральный чанк x 3..9, z -5..1;
  блоки x 50..157, z -78..29 (2 блока запаса).
- Зона критерия мира: новые чанки допустимы только в x -7..19, z -15..11;
  Status-изменения внутри зоны перечислить, не останавливаться.
- Копия v2: `world-template`; `Vanilla::start` каждый запуск удаляет
  `vanilla\world`, копирует шаблон, стирает старый `logs/latest.log` —
  факты из лога копировать до следующего прогона.
- Сервер (наша копия конфигурации): view-distance 10, spectator,
  `network-compression-threshold=-1` (записать значение в отчёте).

## Что осталось (план новой сессии)

1. [ГОТОВО] Добить п.4: execute-пробы из `crates/parity/tests/walk_trace.rs`
   удалены (константы `CHECK_*`/`GM_*`, enum `GmStatus`, `outcome.gm`,
   `probe_err`, чтение лога, секция «Gamemode probes»); вместо них проверка
   `stats.game_mode == Some(3) && stats.abilities_flags == Some(0x07)` —
   не сходится → нарушение → abort последовательности, факты в отчёте
   (mode from the trace) и в walk-summary.md.
2. П.5 ЧАСТИ I: критерий мира через `target/parity/chunk_status.py`
   (`compare <template> <world>`): парсить `only_in_B` (новые чанки) —
   стоп только вне зоны x -7..19, z -15..11; `status_diffs` внутри зоны
   перечислить без стопа; каталог шаблона сверять по SHA; `entities/` и
   `poi/` перечислять отдельно. Region SHA из критерия убрать (остаётся
   информацией).
3. Прогоны П1 (стоять 10 с), П2 (прямая +x), П3 (диагональ +x/-z;
   200 шагов по 0.2, settle 5 с), каждый на свежей копии v2; П2/П3
   запускать автоматически, если предыдущий чист. Условия остановки:
   server `player_position` после первого; кик/ошибка/таймаут при живом
   боте; сработал assert границ; новые чанки вне зоны; режим не spectator.
4. ЧАСТЬ III: разобрать расхождение 173/441 по трейсу П1 и порядку лога
   (без выводов без данных); бот в замерах ждёт 441 чанк или 30 с;
   A) vanilla cold ×3 (свежая копия v2), B) vanilla warm ×3 (второй вход
   на живом сервере), В) наш сервер cold+warm ×3; release-сборка
   `cargo run -p server --release`, порт 25565, vanilla 25566, localhost,
   online-mode off, то же значение float pace. Метрики по монотонному
   времени бота: intention→login(play), →первый level_chunk_with_light,
   →последний chunk_batch_finished, →441-й чанк; число чанков/порций,
   размеры, байты, интервалы; min/median/max по 3 повторам. Записать
   compression threshold. Отметить: vanilla при старте сама подгружает
   область спавна — «холодный» условен; технические различия (сжатие,
   свет, догенерация, кэш payload) — гипотезы; не делать вывод
   быстрее/медленнее без них; выводов по M4.4 не делать.
5. Финальный отчёт (структура владельца): по П1-П3 — время жизни бота;
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
  (только после п.4-хвоста и п.5).
- После правок кода: `graphify update .` (AST-only).
- `docs/PROGRESS.md` обновить после этапа; коммиты Conventional Commits.

## Файлы

- `crates/parity/src/client.rs` — `stamps`/`arrivals`, метки на сокете.
- `crates/parity/src/walk.rs` — бот: таймеры, тики, player_loaded,
  cache-move/порции, факты режима, Stop.
- `crates/parity/tests/walk_trace.rs` — оркестратор 3 прогонов,
  отчёт, diff мира (execute-пробы ещё не удалены — п.4-хвост).
- `crates/protocol/src/packets/play.rs` — `LoginPlay` (game_mode).
- `target/parity/chunk_status.py` — критерий мира (`scan`/`compare`).
- `target/parity/live-session.json` — живой трейс.
- `C:\MineRust-data\reports\packets.json` — идентификаторы пакетов.
- `C:\MineRust-data\traces\` — трейсы, отчёты, `live-seq-51-91.txt`.