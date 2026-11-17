# PROGRESS.md — прогресс проекта

Полный план: `ARCHITECTURE.md` (раздел 8 — этапы M0–M10).
Правила для агента: `AGENTS.md`.

**Текущий этап: M4.5, Шаг 20 — решения владельца по playerdata, разбор parity, план сохранения мира.** Шаг 20a: повторный вход с тем же ником выкидывает старое соединение play-`disconnect` с `multiplayer.disconnect.duplicate_login` (байты сняты с ванильного S15, живой S15 на нашем сервере `2026-11-17_s15-ours-r1` — 0 failed); `save_dir` получает маркер `minerust-save.json` (version 1), `fsync` каталога на Unix; автосейв 6000 оставлен, online-режим отложен. Шаг 20b: разбор C2/C4/C5/C6 — реальные расхождения это отсутствующий `entity_position_sync` (14 пакетов), отставание head yaw в C4 на полшага, состав/темп движения и формат модификатора креатива (строка resource-location, а не UUID); правку сервера не делали. Шаг 20c: план сохранения мира — `docs/WORLD-SAVE.md`, кода нет. Открытые решения — C5/C6 (минимальная правка sync) и формат C6 ждут владельца; §8 `docs/WORLD-SAVE.md` — 7 вопросов по сохранению мира. Ниже — историческая запись M4.5: закрыты 13a–13c (ломание блока), 14b (автоочистка `world.stale`), 14d/14e (плавание), Шаг 15 п.1–п.5 (sprint от `player_command`, `dev_geometry`, сетка G1..G7 — 98.7 %, ползание S11 94/94), шаги 16–19 (трещины, мгновенное ломание, playerdata); реестр — `docs/MECHANICS.md`, планы — `docs/BLOCK-BREAKING.md`, `docs/SWIMMING.md`, `docs/PLAYERDATA.md`, `docs/WORLD-SAVE.md`. Исторические результаты M0–M3 описаны ниже.

Историческая запись M3/Шаг 3.3: коммиты `504de49..192a806` закрыли owner 6-point checklist (live-vs-vanilla отчёт, chunk-diff матрица, причина block entities (7,−2) в `docs/DECISIONS.md`, flake guard и SOURCES/PROGRESS); live проверка 08.11.2026 подтвердила M1/M2/3.3 настоящим клиентом 26.3. Следующий этап M4 был затем принят и выполнен; движок света оставался вне scope этого исторического шага.

## Что сделано

### M0 — Каркас
- [x] Cargo workspace: корневой `Cargo.toml` (`members = ["crates/*"]`,
      `resolver = "2"`, общая `workspace.package`).
- [x] Первый крейт-каркас `crates/protocol` (пустая библиотека с
      doc-комментарием; остальные крейты добавляются по мере этапов).
- [x] CI GitHub Actions: `.github/workflows/ci.yml` — `cargo fmt --all --check`,
      `cargo clippy --workspace --all-targets -- -D warnings`,
      `cargo test --workspace` на Ubuntu и Windows.
- [x] `README.md` с дисклеймером (не официальный продукт Minecraft).
- [x] Лицензия MIT (`LICENSE`).
- [x] `.gitignore`, `.graphifyignore` (`target/` исключён).
- [x] `docs/PROGRESS.md`.

**Готово, когда (M0):** CI зелёный на GitHub — ✅ выполнено: run #1
(https://github.com/tvin4/MineRust/actions/runs/42303431897), оба билда
(ubuntu-latest, windows-latest) — success.

### M1 — Handshake и status
- [x] Графовый хук graphify установлен (`graphify hook install`), первый граф
      собран и закоммичен (`graphify update .`).
- [x] Кодеки в `crates/protocol`: VarInt, строки UTF-16, фрейминг
      `Length + Packet ID + Data` (макс. 2097151 байт, поле длины ≤ 3 байт) —
      `codec.rs`, `frame.rs`.
- [x] Пакеты `crates/protocol/src/packets/`: handshake (0x00), status
      request/response (0x00), ping/pong (0x01) + `StatusInfo::to_json()`.
- [x] Крейт `crates/net` (tokio): `serve()` — accept-loop, обработка
      handshake → status, таймаут соединения 30 s, async `read_frame`/
      `write_frame`. Login/transfer закрывают соединение (реализация — M2).
- [x] Крейт `crates/server`: библиотека (`ServerConfig` из TOML с валидацией,
      `status_info()`, `run()`) + бинарник `main.rs` (`--config`, по умолчанию
      `server.toml`, tracing).
- [x] `server.example.toml` с плейсхолдерами; `/server.toml` в `.gitignore`
      (содержит protocol_version/version_name из игры — в репозиторий не
      коммитится).
- [x] Тесты: 43 зелёных (31 юнит protocol, 4 net, 5 server + 3
      интеграционных в `crates/server/tests/status.rs`: полный SLP
      handshake → status JSON → ping → pong → закрытие, повторные status-запросы,
      закрытие при login-состоянии).
- [x] README: секция «Running the server».

**Готово, когда (M1):** сервер виден в списке серверов клиента 26.3 —
✅ выполнено: smoke-тест бинарника (порт 25565 слушается, конфиг
protocol_version=777 / version_name="26.3" из `version.json` клиентского jar
загружается); проверка в игре — за владельцем.

### M2 — Login и вход в пустой мир

- [x] NBT-кодек сети (тип-байт + payload без имени корня, modified UTF-8,
      лимит глубины 512) + конвертация data-pack JSON → NBT для записей
      реестров (`protocol/src/nbt/`). Смешанные массивы Float+Double
      промотируются в List<Double>; единственный такой случай во всём jar.
- [x] Пакеты login-состояния: Login Start/Success, Login Acknowledged,
      Set Compression, Disconnect + offline UUID v3 (MD5 от
      `OfflinePlayer:<name>`, как задокументировано в публичной документации
      протокола).
- [x] Крейт `crates/data`: `load_from_jar`/`load_from_zip` читают
      `data/minecraft/` из клиентского jar владельца — 29 синхронизируемых
      реестров (записи отсортированы по идентификатору → детерминированные
      id) и теги, разрешённые в числовые id (вложенные теги, необязательные
      ссылки, циклы — ошибка). В репозиторий данные не кладутся.
- [x] Конфиг: обязательное поле `client_jar` (`deny_unknown_fields`,
      валидация на непустую строку); `server.example.toml` обновлён.
- [x] Пакеты configuration-состояния: Registry Data (один реестр на пакет,
      порядок записей = id), Update Tags (только синхронизируемые реестры,
      пустые теги не отправляются), Finish Configuration/Ack, Client
      Information.
- [x] `net`: login → configuration → play. Login Success шлёт offline UUID,
      configuration отправляет все реестры, теги и Finish Configuration;
      play — join-последовательность Login (0x32) → Set Default Spawn
      Position (0x63) → Player Info Update (0x47) → Set Center Chunk (0x60)
      → Game Event 13 (0x27) → Synchronize Player Position (0x49, teleport_id
      1, flags 0), затем keep-alive каждые 10 с (cb 0x2D / sb 0x1C; в
      части 2 замерен интервал ванили — 15 с).
- [x] Пакеты play-состояния в `protocol` + тип Position (x26/z26/y12),
      Int/Float/Double в курсоре.
- [x] Таймаут 30 s — только до play (handshake, status, login,
      configuration); в play — кик по неответу keep-alive (реализован в
      части 2, п.2.3). Чтение play-фреймов идёт в фоновой задаче через
      mpsc-канал, чтобы ветка keep-alive не отменяла наполовину
      прочитанный фрейм.
- [x] Неизвестные serverbound-пакеты в play логируются и игнорируются
      (соединение не рвётся); plugin message — лог канала, payload
      игнорируется.
- [x] Тесты: 114 зелёных. Интеграционные: `status.rs` (SLP + полный offline
      login до play с проверкой всех пакетов join-последовательности),
      `join.rs` (полный handshake с fixture-jar: все 29 реестров ровно один
      раз, записи в порядке id, теги → числовые id, dimension_type id
      берётся из реестра). Fixture-конструктор jar — `data::fixtures`
      (feature `fixtures`), общие хелперы — `crates/server/tests/common`.

**Готово, когда (M2):** клиент 26.3 заходит в пустой мир и не вылетает —
код готов (offline login без шифрования, Variant A без Known Packs,
без сжатия); ручная проверка в игре — за владельцем (см. «Что дальше»).

### Пред-M3 аудит источников данных (выполнен)

- [x] `docs/SOURCES.md`: аудит всех пакетов, констант и поведений M1–M2 —
      источник (jar / генератор / вики / догадка) и статус проверки
      (авто отчёт / auto fixture / jar / живой клиент / не проверено).
      Главный вывод: все 31/31 ID пакетов совпадают с реальным
      `reports/packets.json` 26.3, живой клиент пока не подтверждал ничего.
- [x] Пункт 2: ID пакетов берутся из отчёта генератора. `protocol`:
      `PacketKey` (state, direction, имя, компилируемый дефолт) + `KEY`-константы
      у всех пакетов, `used_packet_keys()` (31 ключ). `data`:
      `load_packets_report`/`packets_from_json`/`PacketIds` (ошибки
      `ReportRead`/`MissingState`/`MissingPackets`). Конфиг:
      опциональное `packets_report` (валидация, загрузка до бинда порта,
      `verify()` = fail-fast), без него — совпали компилируемые дефолты в
      лог warn. `net::serve` несёт `Arc<PacketIds>`, все сравнения/записи —
      `ids.id(KEY)`, локальные ID-константы удалены.
      Отчёт в репозиторий не коммитится (env/путь конфига).
- [x] Тесты сверки: юнит (fallback, override, отсутствие состояния/пакета) +
      ignored `compiled_packet_ids_match_the_real_report` (env
      `MINECRAFT_PACKETS_REPORT`) + ignored по `MINECRAFT_JAR`; интеграционный
      `status_response_comes_from_the_packets_report` доказывает, что сервер
      шлёт ID из отчёта (overridden `StatusResponse`).
      Всего тестов: 132 (было 114).
- [x] Пункт 3: `// SOURCE: wiki, unverified` на непроверенных местах
      (фрейм/строковые лимиты, VarInt, offline UUID, порядок джоина,
      event 13, тайминги, view/sea level, маска player-info, адрес handshake).
- [x] Пункт 4: в `AGENTS.md` — порядок источников истины
      (jar/генератор → парити → вики как подсказка).
- [x] Пункт 5: план крейта `parity` в `docs/SOURCES.md`; предложение —
      первым шагом внутри M3.
- [x] `tools/generate-reports.ps1` — сбор classpath из версионного json
      лаунчера, @argfile, запуск `net.minecraft.data.Main --reports`
      (проверено на машине владельца: 43 entry, отчёт сверен).

### M3, шаг 0: окружение ванильного сервера (выполнено)

- [x] Официальный `server.jar` 26.3 скачан через манифест Mojang
      (`piston-data.mojang.com`, sha1 `33680f5f…` сверен) в
      `E:\MineRust-data\server.jar` (вне репозитория).
- [x] `E:\MineRust-data\vanilla`: первый запуск создал `eula.txt` и
      `server.properties`; EULA принят владельцем → `eula=true` (единственная
      правка файла). `server.properties`: `level-seed=12345`,
      `online-mode=false`, `server-port=25566`, `view-distance=10` (дефолт),
      `level-type=minecraft:normal`, дополнительно `white-list=false`
      (дефолт 26.3 — true, блокировал бы offline-вход тест-клиента).
- [x] Мир сгенерирован с сидом 12345 — проверено разбором gzip+NBT: тег
      `seed` (long) в `world/data/minecraft/world_gen_settings.dat` = 12345
      (в `level.dat` 26.3 сида больше нет). Мир сохранён чисто (`stop`,
      exit=0) и скопирован в шаблон `E:\MineRust-data\world-template`.
- [x] `server.toml` (gitignored): добавлены `vanilla_server_jar` и
      `vanilla_server_dir` (оба пути — вне Git). `ServerConfig` получил
      опциональные `vanilla_server_jar` /
      `vanilla_server_dir` (валидация непустых, тесты парсинга);
      `server.example.toml` — закомментированные примеры.
      `git check-ignore` подтверждён: ни `server.toml`, ни `E:\MineRust-data`
      в Git не попадают. Тестов: 135 (было 132).

### M3, шаги 1–3: крейт parity и срезы status/join (выполнено)

- [x] Шаг 1 — каркас: `crates/parity` с `env.rs` (путь к `server.toml` через
      `MINECRAFT_SERVER_TOML`/корень workspace) и `vanilla.rs`
      (`Vanilla::start/stop`: мир на прогон копируется из `world-template`,
      ожидание `Done (` в `logs/latest.log`, `stop` по stdin,
      kill_on_drop). Ignored-тест `vanilla_server_starts_and_stops`
      проверен вживую (~12 s, exit 0).
- [x] Шаг 2 — скриптованный тест-клиент: `trace.rs` (raw-трейсы
      `(state, direction, id, payload)` в JSON), `client.rs`
      (шаги Send/Wait/Respond/Quiet, фреймы с поддержкой Set Compression),
      `tests/status_slice.rs`: трейсы status для ванили и нашего сервера →
      `target/parity/status-{vanilla,ours}.json`.
- [x] Шаг 3 — join-срез `tests/join_slice.rs`: handshake → login →
      configuration → play до тишины; трейсы →
      `target/parity/join-{vanilla,ours}.json` (ваниль 1799 пакетов,
      наш 14). Вживую оба сервера прошли сценарий.
- [x] Настройка ванили по факту: ответ на пробу `select_known_packs`
      — отдельный сервербаунд-пакет `minecraft:select_known_packs` (id 7 из
      `reports/packets.json`), не `custom_payload` (эхо в custom_payload
      роняло ваниль с «Internal Exception»). Клиент также автоматически
      подтверждает play keep-alive (иначе ваниль кикает по ~30 s таймауту);
      эхо known packs помечено `// SOURCE: wiki, unverified` до
      подтверждения прогоном.
- [x] Новые ключи `protocol`: `SELECT_KNOWN_PACKS` (0x0f cb),
      `SELECT_KNOWN_PACKS_SERVERBOUND` (0x07 sb), `UPDATE_ENABLED_FEATURES`
      (0x0d cb), `CONFIGURATION_CLIENTBOUND_PLUGIN_MESSAGE` (0x01 cb) —
      `used_packet_keys()` теперь 35. Тестов (без ignored-срезов): 151.

- [x] Дифф-движок `src/diff.rs`: группировка `(state, direction, id)`,
      сравнение счётчиков и сырых байт n-ных вхождений; нормализация
      случайностей строго ограничена двумя правилами (порт handshake —
      эфемерный порт тестового сервера, teleport id — растёт на
      соединении). Имена/UUID/координаты/entity id НЕ нормализуются —
      это и есть объект спора. Ignored-тест `tests/diff_report.rs`
      строит отчёты → `target/parity-report/{status,join}-diff.md`.
- [x] Прогнаны отчёты: status — 1 расхождение payload (формат и текст
      description в status_response); join — 43 строки состава (ваниль
      1799 пакетов против наших 14: чанки, теги, реестры, keep-alive) и
      7 расхождений payload из 14 сопоставленных л.с.

### M2-фикс по живому прогону: registry_data и update_tags из данных игры (выполнено)

Живой клиент 26.3 упал на «Registry Loading: Failed to load registries due to
errors» (не распарсились dimension_type/enchantment/sulfur_cube_archetype из-за
ссылок на неотправленные теги, unbound dialog-теги, «Static Registries» пусты).
Задачи владельца (a)–(e):

- [x] (a) Инспектор `crates/parity/tests/trace_inspect.rs` (ignored): таблицы
      «до» по трейсу ванили. registry_data: **ваниль 32, у нас 29** —
      отсутствуют `minecraft:block_transformer`,
      `minecraft:decorated_pot_pattern`,
      `minecraft:worldgen/block_state_provider` (их нет в
      `SYNCHRONIZED_REGISTRIES` — это статичные реестры, которые в 26.3
      ваниль всё равно шлёт в Registry Data; править по задаче (f)/для
      registry_data — отдельно). update_tags: **ваниль 15 реестров /
      у нас 7**; отсутствовали block (300 тегов), item (236), entity_type
      (49), fluid (10), game_event (5), point_of_interest_type (3), potion
      (5), dialog (2, при этом оба — пустые) + damage_type 35/36 (его 36-й
      тег пустой — `resolve()` отбрасывал пустые теги).
- [x] (b) `registries_report` в `server.toml` (путь к
      `reports/registries.json` генератора, как `packets_report`): крейт
      `data` — модуль `registry_ids` (95 реестров → protocol_id статических
      записей, ошибки `RegistriesReportRead`/`EmptyReport`). Правило отбора
      теговых реестров: реестр берётся из `registries.json` **или** из
      `SYNCHRONIZED_REGISTRIES` → ровно ванильские 15: папка тегов
      `villager_trade` (73 файла) и `worldgen/{feature,structure,world_preset,flat_level_generator_preset}`
      исключаются — их нет ни в отчёте, ни в списке синхронизации.
      Id тегов: синхронизируемые реестры — позиция записей Registry Data,
      статические — protocol_id из отчёта (`load_from_zip` собирает
      `entry_ids`, синхронизация перекрывает отчёт). Пустые теги больше не
      отбрасываются (dialog 2/0, damage_type 36/230 — как у ванили).
      `server.example.toml` и `tools/generate-reports.ps1` обновлены
      (скрипт подсказывает и `registries_report`).
- [x] (c) Ignored-тест `crates/parity/tests/update_tags_parity.rs`: конструктор
      пакета вынесен в `GameData::tags_packet()` (боевой путь `connection.rs`),
      тест сравнивает его с пакетом ванили из трейса: **состав и значения
      совпадают полностью — 15 реестров, 773 тега, все id построчно равны**.
      Полная байтовая сверка payload выполняется только при совпадении
      порядка; порядок ванили (реестры в пакете и теги внутри реестра) —
      итерация внутренних мапов сервера, правило её воспроизведения в
      трейсе не выводится (см. «Известные проблемы»), поэтому отклонение
      порядка — предупреждение, а не падение.
      Проверка в реальном времени запущена: `tag_registries=15` в логе при
      старте.
- [x] (e) `server.toml` (gitignored, локально владельца) — прописаны
      `packets_report` и `registries_report`; warn'ов при старте нет:
      лог `registries report loaded (95)` → `game data loaded:
      registries=29, tag_registries=15` → `packet ids loaded ... ids=260`.
- Тестов: 160 зелёных (было 151).

**Стоп владельца (f): остальные расхождения таблицы среза 1 и срез 2
(чанки) не трогать до подтверждения.**

### M2-фикс 2: три недостающих реестра в registry_data (выполнено)

Вторая живая ошибка: клиент 26.3 упал на `finish_configuration` с
«Missing element ResourceKey[minecraft:block_transformer / minecraft:shovel]»
— ваниль шлёт в Registry Data 32 реестра, у нас было 29.

- [x] Содержимое трёх реестров найдено **в клиентском jar** (не из трейса):
  `data/minecraft/block_transformer/` (axe, hoe, shovel — 3 файла),
  `data/minecraft/decorated_pot_pattern/` (23 файла),
  `data/minecraft/worldgen/block_state_provider/` (8 файлов). Генератор
  данных (`reports/registries.json`) этих реестров не содержит — он даёт
  только protocol_id статических реестров; трейс ванили использован только
  как эталон сверки.
- [x] `SYNCHRONIZED_REGISTRIES` в `crates/data/src/load.rs`: 29 → 32 записи.
      Данные игры в репозиторий не коммитятся (по-прежнему только из jar).
- [x] Ignored-тест `crates/parity/tests/registry_data_parity.rs`: сравнивает
      с трейсом ванили имена реестров, ключи записей, их количество и
      порядок по всем 32 реестрам → **совпало полностью: 32 реестра,
      432 записи, порядок записей ванильный** (порядок реестров в пакете —
      наш алфавитный, у ванили порядок итерации мапов, осознанное отличие
      в `docs/DECISIONS.md`).
- Старт сервера чистый: `game data loaded (registries=32, entries=432,
  tag_registries=15)`.
- Тестов: 161 зелёный. Проверено также, что теги для этих трёх реестров в
  jar отсутствуют — состав update_tags не изменился (15 реестров).
- Graph: `graphify query "содержимое реестров... + load_from_jar"` →
  `crates/data/src/load.rs` (SYNCHRONIZED_REGISTRIES, load_registry),
  `crates/protocol/src/packets/configuration.rs` (RegistryData),
  `AGENTS.md`, `docs/SOURCES.md`, `docs/PROGRESS.md`.

### Часть 2 — правки по таблице расхождений среза 1 (п.2.1–2.5 выполнено)

Каждый пункт — отдельный коммит, в теле — ссылка на строку/секцию
`target/parity-report/*-diff.md`:

- [x] (`88f619f`) **2.1** — нормализация в парити (entity/teleport id →
      `NORMALIZED_ID` в первом VarInt, `intention` порт), `PacketIds::names_by_id()`
      для отчёта, motd в `server.toml` = ванильскому, compression=-1 в
      `server.properties` ванили, GameData из jar для join-среза.
- [x] (`412454a`) **2.а status_response** — description = обычная строка,
      порядок `description,players,version`: status-срез **«Трейсы совпадают,
      0 из 5»** (payload = байт-в-байт).
- [x] (`aef73eb`) **2.b login_finished** — первые 41 байт уже совпадали
      (v3-profile uuid в трейсе идентичен), хвост = session uuid: ваниль на
      соединение рисует **случайный v4** (два прогона: `4d52e996...` /
      `0a00ceb2...`), у нас повторялся profile uuid → `random_session_uuid`
      (getrandom) + нормализация хвоста в diff (как порт):
      `ConnectionError::Entropy`.
- [x] (`188a0e2`) **2.c login (play)** — раскладка полей 1:1 с ванилью;
      entity id (4 vs 1, счётчик соединения) нормализован первыми 4 байтами;
      `dimension_names` = размерности **мира** (в трейсе ванили 3 — без
      `overworld_caves`, порядок имён между прогонами нел → тест сравнивает
      как множество), у нас = `[overworld]`; `hashed_seed(seed) = i64 LE из
      sha256(seed.to_le_bytes())[..8]` — проверено по проводу (`12345` →
      `04139124513554E1` = мир `level-seed=12345`); сам seed ждёт level.dat
      (часть 3). Ignored-тест `login_fields.rs` декодирует трейс ванили нашим
      кодеком и проверяет каждое поле. Dep `sha2`.
- [x] (`5c960c9`) **2.d player_info_update** — маска = raw **u8 `0xFF`**
      (8 действий), per-player: name, props, init-chat `00`, gamemode,
      listed, latency, display `00`, session `00`, восьмой байт `01`
      (семантика восьмого действия не проверена — воспроизводится как
      записана; присутствующие опции теперь отклоняются явной ошибкой
      `CodecError::Unsupported`, а не парсятся мимо). Ignored-тест
      `player_info.rs`: encode == записанные байты ванили — проходит.
- [x] (`791ee90`) **2.3 keep-alive** — интервал **15 с из трейса**
      (разница id `0x0220A0CF − 0x02206633` = 15 004 мс), id = монотонные мс
      (якорь — старт сервера; у ванили это аптайм машины ≈35.6М, origin
      несопоставим → не байт-сравнится), `KeepAliveWatch` + кик
      `ConnectionError::KeepAliveTimeout` через 30 с
      (`// SOURCE: wiki, unverified` — таймаут в трейсе не наблюдаем).
- [x] (`8ccb079`) **2.4** — `docs/DECISIONS.md`: Known Packs **не**
      реализуем (реестры целиком), сжатие отложено (у ванили парити-прогонов
      `network-compression-threshold=-1`).
- [x] **2.5** — `docs/SOURCES.md`: «проверено parity» только для сошедшегося
      (status_response байт-в-байт, login_finished структура + random v4,
      login-поля layout/hashed_seed, player_info bytes, интервал keep-alive)
      + обновлён `docs/PROGRESS.md`.

- Тестов без ignored-срезов: 168 зелёных; `fmt`/`clippy -D warnings` чистые.

## Часть 3, шаг 3.0: обследование шаблона мира (выполнено)

- [x] `crates/parity/tests/world_inspect.rs` (`#[ignore]`, запуск
      `cargo test -p parity --test world_inspect -- --ignored --nocapture`,
      вывод в `target/parity/world-inspect.txt`): NBT-деревья `level.dat`,
      `world_gen_settings.dat`, `game_rules.dat` + ведомость всех `.mca`.
  - Шаблон: 4 region-файла, **529 чанков** в квадрате 23×23 (x −5..17,
    z −13..9), все zlib, `DataVersion=5023`, `Version.Name = "26.3"`.
  - Корень чанка: `DataVersion, Heightmaps, InhabitedTime, LastUpdate,
    PostProcessing, **Status** (с большой буквы), block_entities,
    block_ticks, entities, fluid_ticks, sections(24), structures,
    xPos, yPos, zPos`.
  - `Status` по чанкам: `structure_starts` 480, `biomes` 24, `terrain` 16,
    `initialize_light` 8, `full` 1; палитры блоков и биомов — имена (+свойства)
    в самом NBT (подтверждение правки 2 плана: для сети нужен маппинг на
    state id, т.е. `reports/blocks.json`).
  - **Свет (`BlockLight`/`SkyLight`) есть только у 9 из 529 чанков** — ровно
    3×3 вокруг спавна (5..7, −3..−1); эти же 9 чанков ваниль прислала первым
    батчем при джоине.
- [x] (правка плана 4, эксперимент) Позиция игрока **плавала** между
      прогонами: `(89.5,131,−24.5)` → `(91.5,140,−32.5)` (центр чанков
      `(5,−2)` → `(5,−3)`), при одинаковом `set_default_spawn_position` из
      level.dat `[96,145,−32]`. Имя правила найдено в данных 26.3
      (реестр `minecraft:game_rule`): `minecraft:respawn_radius` = 10 →
      обнулён в `game_rules.dat` шаблона (бэкап в temp). После: **2 прогона
      из 2** дают `(96.5,145,−31.5)` (= блок спавна), центр `(6,−2)`,
      порядок 9 чанков байт-стабилен. Diff хэшей шаблона до/после прогона:
      менялся **только** `game_rules.dat` (правка), прогоны шаблон не трогают
      (`prepare_world` копирует его в рабочую копию ванили).
- [x] (правка плана 3, по трейсу) Первый батч — **9 чанков**, бот запрашивает
      view distance **8** (до 17×17 = 289 потенциально): `9 ≠ весь радиус`,
      плюс тело `chunk_batch_finished` = `09` = размеру пачки. Ограничение:
      9 = ровно число «готовых» чанков (со светом), поэтому одно число не
      разделяет гипотезы «окно первой порции» vs «что было готово»; sb
      `chunk_batch_received` в трейсе нет — бот-клиент его не шлёт (все 8 sb
      — логин/конфигурация/keep-alive).
- **Правки плана учтены:** `blocks.json` отсутствует в `E:\MineRust-data\
  reports` — блокер палитры (правка 2); chанк другой версии → явная ошибка
  (6); `set_chunk_cache_center` — две VarInt, расхождений нет (1); свет и
  `light_update` (открытые вопросы 1–2) — данные выше; отсутствующих чанков
  отправлять не будем (5).

## Часть 3, шаги 3.1 и 3.2: crates/world и раскладка payload (выполнено)

- [x] **Шаг 3.1 — крейт `crates/world`:** region-ридер `.mca` (заголовок
      секторов, zlib/gzip/raw, проверка длины) и модель чанка/секции
      (Block Light/Sky Light, палитры блоков и биомов из NBT шаблона),
      загрузка `blocks.json` (имя+свойства → state id). Юнит-тесты зелёные;
      структура/документация под `cargo clippy -D warnings`.
- [x] **Шаг 3.2 — раскладка `level_chunk_with_light` из трейса ванили**
      (ignored-тесты `parity`, вывод в `target/parity/section-map.txt`,
      `chunks-open.txt`):
  - **Payload (45852 байта для (6,−2)):** `x,z` BE i32 @0 → карты высот
    @8..**903** → VarInt длины sections-blob @903 (32581, 3 байта) →
    blob 906..33487 → block entities @33487 → свет @33550..45852.
    **`03 04 25` расшифровано:** после x,z — VarInt счётчик карт высот
    (3 записи `[id]["25"]=VarInt 37 longs`); `04`/`05`/`01` — id записи.
    Во всех 9 чанках карты кончаются ровно на байте 903 (тест `chunks_open`).
  - **Запись секции:** `[u16 non-air][u16 fluid-счётчик][VarInt bits]
    [VarInt count палитры][count×VarInt state-id][data (BE i64, без
    length-преамбулы)]` затем биомы `[VarInt 0][VarInt id]` либо
    `[bits][count][ids][data]`. Длина data = `ceil(4096/(64/bits))*8`:
    bits 4→2048, 5→2736, 6→3280 байт — все 14 секций сходятся (assert);
    bits = 4/5/6 по размеру палитры (≤16/≤32/больше).
  - **Сверки (все зелёные):** `state ids equal: true` и
    `block names equal: true` для всех 14 секций (wire ↔ NBT, вариации
    `sculk_vein`/`leaf_litter`/`dark_oak_leaves` резолвятся через
    `blocks.json`); non-air, посчитанный из data, равен u16-заголовку;
    биомы (deep_dark=11, dark_forest=9, lush_caves=31, plains) байт-в-байт.
  - **Свет NBT ↔ wire — расхождение (документировано):** точные близнецы
    (2048/2048) — BlockLight Y−3/Y−2 и SkyLight Y9/Y10; слот Y8 и один
    block-слот отличаются — свет менялся между джоином и сохранением
    шаблона (NBT хранит свет только для Y=[−3,−2]).
  - **Утечки палитр не было:** сырой дамп `raw_palettes.rs` видит все
    19/25/20 элементов списков; причина расхождений была в парсере
    (форма `{"id", "properties"}`) и в ложном кадре палитры.
- [x] Правки владельца к 3.0 учтены: правило ChunkReady по данным
      (`DataVersion=5023`, `Status` ∈ {`full`, `initialize_light`}, есть
      хотя бы один BlockLight/SkyLight → 9 пригодных, 520 остальных → M6),
      `ChunkNotReady(status)`, свет из NBT — в `docs/DECISIONS.md`,
      `blocks_report` в `server.toml` (блокер снят: отчёт сгенерирован),
      бэкап `game_rules.dat.orig`, guard флейка `join_slice`.
- Открытые точки шага 3.2 (сняты проверками ниже): block entities в
  payload и id карт высот.

## Часть 3, шаг 3.2½: три проверки перед 3.3 (выполнено)

Четыре ignored-теста `parity` (вывод в `target/parity/*.txt`), все
зелёные (`cargo fmt`, `clippy -D warnings`, `cargo test --workspace`):

- [x] **Проверка 1 — `blob_walk.rs`:** ровно 24 секции NBT (Y −4..19:
  14 packed + 10 single-valued), блоб 32581 Б пройден целиком —
  `cursor 33487 == blob end 33487 (delta 0)`; длины data 2048/2736/3280
  и fluid-заголовок `Y2=124` сверены. Семантика второго u16 = число
  клеток воды — одна наблюдаемая точка, `// SOURCE: wiki, unverified`.
- [x] **Проверка 2 — `block_entities.rs`:** формат
  `[u8 packed xz][i16 BE y][VarInt тип][NBT]` подтверждён на всех 10
  записях чанка (6,−2): координаты 10/10, типы 10/10 (из
  `registries.json`), NBT сверен с чанком (9× `00`, один real NBT у
  (5,−1) — `mob_spawner`, 175 Б). Фиксированный шаг 5 Б валиден
  только при пустом NBT.
- [x] **Проверка 3 — `heightmap_ids.rs`:** реестра `heightmap_type` в
  `registries.json` **нет** (0 совпадений); id разрешены байтовым
  пересечением по чанкам: **1 = WORLD_SURFACE, 4 = MOTION_BLOCKING,
  5 = MOTION_BLOCKING_NO_LEAVES**. 7 wire-записей не равны ни одной
  хранимой карте чанка — отпечатаны в `heightmap-ids.txt`,
  не валят тест; точка для 3.3f.
- [x] **Бонус — `light_framing.rs`:** фрейминг света закреплён данными
  всех 9 чанков: `[4 BitSets (sky/block/emptySky/emptyBlock)]` →
  `[VarInt count][count × (VarInt 2048 + 2048 Б)]`, `popcount(маски) ==
  числу массивов`, пустые и занятые маски не пересекаются, конец
  payload точен у всех 9.

Решения записаны в `docs/DECISIONS.md`: (а) критерий парити 3а — сырые
байты payload совпадают полностью, **кроме световых массивов**; свет
сравнивается по структуре, расхождения — отдельным отчётом без падения
теста; движок освещения — отдельный этап; (б) секция чанка без света в
NBT → sky 15 / block 0, временное решение (не parity), помечено в
`docs/SOURCES.md` (таблица 4).

## Часть 3, шаг 3.3: маленькие коммиты a–g (выполнено, СТОП)

Перед каждым коммитом: `cargo fmt`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace` — чисто
(медленные и env-зависимые тесты — `#[ignore]`, без env печатают SKIP).

- [x] **a) `504de49` — кодеки `level_chunk_with_light`, `chunk_batch_*`,
      `set_chunk_cache_center`** из записанных payload: раскладка полей
      вычитана из трейса (никакой вики), round-trip-тест `chunk_codec`
      (`cargo test -p parity --test chunk_codec -- --ignored`):
      `encode(decode(bytes)) == bytes` для всех 9 чанков записи.
- [x] **b) `b04efec` — правило готовности:** `Chunk::check_sendable` /
      `load_sendable_chunk`, причина отказа именованная
      (`ChunkNotReady(status)` и др.), правило в `docs/DECISIONS.md`;
      9 пригодных из 529, остальное — список «ждёт генерации (M6)».
- [x] **c1) `e25f58d` — конвертация сохранённого чанка в payload**
      (`world::wire`): heightmap-записи под id, сверенные на трейсе;
      палитры блоков/биомов через `WireRefs` (state id из `blocks.json`);
      блочные сущности; свет по §141.
- [x] **c2) `7d2f884` — актор-загрузчик:** `net::loader` — пул из 2
      `std::thread`-воркеров; связь с миром только через трейт
      `net::loader::ChunkSource` (крейт `net` **не** зависит от `world`);
      `server::build_world` → `(Option<Loader>, net::WorldInfo)`. Ни одной
      новой блокировки в тике.
- [x] `1311182` и `7e0b6ac` (между шагами): keep-alive-тест стабилизирован
      (четвёртый медленный → `#[ignore]`), `connection.rs` разбит на
      модули (`connection::play/...`) — снят лимит длины файла.
- [x] **e) `40351a0` — play-поля из сохранения:** спавн из `level.dat`
      (`Data.spawn.pos` = `[96,145,−32]`), `hashed_seed` из
      `world_gen_settings.dat` (seed 12345), центр кэша чанков и стартовая
      позиция — оттуда же; тесты `build_world_reads_the_saved_spawn_and_seed`
      и два на отсутствующий/битый документ мира.
- [x] **d) `686a509` — пейсинг `chunk_batch_received`:** sb-пакет (KEY из
      отчёта, ID 0x0B, одно поле Float), `ChunkPace` (`ack`,
      `sub_batch_size`), подбатчи уходят `write_sub_batch` с паузой 50 мс;
      общий `parity::join_steps` + ignored-тест `chunk_batch_slice`
      (прогон против ванили — за владельцем).
- [x] **f) `192a806` — парити-срез `chunk_slice` + таблица
      `target/parity-report/chunk-diff.md`:** сверяется запись
      `join-vanilla.json` с тем, что наш продакшн-путь (`TemplateWorld` +
      `WireTables`, экспортированы из `server`) конвертирует из шаблона;
      15 полевых групп, файлы разбиты на `chunk_slice.rs` +
      `common/chunk_compare.rs` (лимит длины). Срез офлайн (без ванили),
      прогонял локально: **9/9 чанков сопоставлено** — равны id карт
      высот (9/9), число секций (9/9), палитры биомов (9/9); слова карт
      высот 5/9, заголовки non-air/fluid 1/9, блочные палитры 1/9
      (палитра равна, data-слова нет), block entities: счётчик 8/9,
      записи 5/9, NBT 8/9 (в (7,−2) ваниль шлёт 2 записи, у нас 0);
      **свет 0/9 по всем 4 маскам и обеим группам массивов** (у (6,−2):
      vanilla sky `[13,14,15]` / 3 массива, у нас 24 бита / 24 массива —
      это синтез §141, см. `DECISIONS.md`). Таблица ничего не «чинит»:
      решение по её полям за владельцем.
- [x] **g) документация (этот коммит):** `SOURCES.md` (спавн подтверждён
      байтами трейса, seed из `world_gen_settings.dat`, §141 — 5 из 24
      секций с сохранённым светом в (6,−2), строка `chunk_slice`, статусы),
      `DECISIONS.md` (§141 «Проверка» исправлен: секции без сохранённого
      света реально есть — остальные 19 из 24 в (6,−2)), `PROGRESS.md`,
      `graphify update`, push.

## Часть 3, шаг 3.3: живая проверка — диагностика по 6 пунктам (выполнено; блокер п.6 снят `21c5d1d`, вход клиента подтверждён 08.11.2026)

Живая проверка провалена (клиент ~30 с в списке серверов → пустота/тишина).
Вместо слепых починок сделана диагностика; игровой код **не менялся**
(единственное исключение — fail-fast при отсутствии путей, разрешённое
владельцем). Что сделано по шести пунктам:

1. **Конфиги + fail-fast + старт-лог:** `require_chunk_paths` в `run()`
   (четыре пути: `world_dir`, `blocks_report`, `registries_report`,
   `packets_report`; первая недостающая → `RunError::MissingSetting` с
   подсказкой), `build_world` стал `pub` (lenient), прогревочный
   `warm_world` пишет info-лог загрузчика; `server.toml` и
   `server.example.toml` помечены REQUIRED.
2. **Логи чанков/батчей/пакетов:** info — `chunk batch requested from the
   loader`, `world read finished, chunks found`, каждая
   `level_chunk_with_light sent` (x/z/размер), `chunk_batch_finished sent`,
   `chunk batch sent`, `chunk_batch_received: pace chosen`; warn —
   `loader=None` и чанк вне радиуса (`outside_view`); debug —
   `chunk_batch_start sent` и каждый пакет из `trace::record`
   включить: `RUST_LOG=net=debug`).
3. **Трейс живого сеанса:** `MINECRAFT_SESSION_TRACE=1` (или путь) →
   `target/parity/live-session.json`; формат — заголовок + JSON-записи с
   запятыми *между* ними, чисто закрытый файл — валидный JSON, хвост
   убитого сервера чинит `salvage_tail`; payload > 64 КБ писался `null` —
   поднят до 1 МиB (порт в этом шаге), запись реального сеанса
   08.11.2026 работает (блокер п.6 снят).
4. **Сравнение с ванилью:** `parity/tests/live_vs_vanilla.rs` (без assert,
   отчёт `target/parity-report/live-vs-vanilla.md`): порядок login→первый
   чанк, serverbound в порядке чтения, чанки/центр. Предварительный
   прогон на `join-{vanilla,ours}.json`: у ванили
   `set_chunk_cache_center` = `06 feffffff0f` → **центр (6,−2) подтверждён
   на проводе**, спавн (96,145,−32) → чанк (6,−2) под (96.5,145,−31.5) ✓.
5. **Свой сеанс:** `parity/tests/self_session.rs` (`--ignored`) поднимает
   наш сервер in-process с реальным миром и скриптованным клиентом;
   assert'ы: центр (6,−2), ровно 9 чанков, (6,−2) среди них, ничего вне
   центр±10. На блокере п.6 ронялся на чтении мира; блокер снят — тест
   проходит до конца.
6. **Блокер п.6 (`seed` внутри compound `data`) снят** — фикс `21c5d1d`
   (fallback на обе формы документа), аудит документов `b9e89bf`; серия
   «диагностика → фикс → живая проверка» закрыта входом настоящего клиента
   26.3 (M1/M2/3.3 подтверждены 08.11.2026).

## Часть A плана M4: шаблон v2, эталоны, радиус отправки (выполнено, СТОП — живую проверку делает владелец)

Пункты 1–7 закрыты; числа, выводы и команды воспроизведения — в
`docs/DECISIONS.md`, секция «Часть A плана M4».

1. **Шаблон v1 зафиксирован**: sha256-манифест 26 файлов
   (`E:\MineRust-data\world-template-v1.sha256.txt`, агрегат
   `2f6c0cd4…b00d41`), копия `E:\MineRust-data\world-template-v1`,
   бэкап 8 старых трейсов `target/parity/*.v1diag.json`.
2. **`world-template-v2` создала ваниль**: прогон `join_slice` на копии,
   штатный `stop`, без forceload, статусы читались из `.mca`
   (`target/parity/chunk_status.py`), не по времени. Итог: 36 файлов /
   15 402 135 байт, заменил активный `world-template`; готово **729**
   чанков из 2209 (`full` 625, `initialize_light` 104).
3. **Данные п.3:** гипотеза «свет есть в NBT у всех секциях full-чанков»
   **опровергнута** (BlockLight 3673/15000 = 24.5%, SkyLight 2355/15000 =
   15.7%, полных чанков — 0), поэтому подстановка `sky 15` остаётся, а
   свет сравнивается отчётом `chunk-diff.md`. Карты высот между двумя
   прогонами: наборы и статусы идентичны (2209 = 2209), различаются 5
   чанков вне спавна.
4. **Эталоны перезаписаны на v2**: run A (30 116 пакетов) и run B
   (31 443), снапшоты `E:\MineRust-data\world-v2-run{A,B}`, эталон =
   run B. `chunk_slice` прогон →
   `target/parity-report/chunk-diff.md`: блоки/палитры/биомы/BE-NBT/
   слова высот 9/9, non-air 5/9, порядок id карт высот 0/9 и записей BE
   6/9, свет 0/9 (block-массивы 9/9 — исключение).
5. **П.5 не применим:** условие «свет в NBT у всех секций» не выполнено
   (п.3), поэтому подстановка `sky 15` не убирается — результат показан
   отчётом, как и договорились.
6. **Правило радиуса закодировано в `crates/net`:** R = min(клиент 8,
   сервер 10, готовые 729) = **8** → отдаём **289** чанков (без фильтра
   было бы 729, из них 288 за пределами ±10 и 440 за ±8). `Client`
   пробрасывает `view_distance` из configuration в play,
   `write_chunk_batch` фильтрует и логирует скипнутые; тесты `net` зелёные.
7. **`self_session` обновлён под v2:** сначала ждёт `chunk_batch_start`
   (на v2 loader собирает 729 payload дольше старой паузы «2 с тишины» —
   потому тест раньше получал 0 чанков), ожидание — 289 чанков квадрата
   ±8, центр (6,−2), (6,−2) среди них, ничего вне радиуса. Проходит за
   ~29 с.

Гейт перед коммитами зелёный: `cargo fmt`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace`.

**Дальше:** владелец проводит живую проверку Части A; после его
подтверждения — Часть B (`docs/ARCH-M4.md`, ждать «дизайн принят»,
затем M4.1).

## Часть A2 (п.1–5): лог server — порядок чанков, кэш батча, уровни лога, дистанции, пейсинг (выполнено, СТОП)

Пять задач владельца, каждая — отдельный коммит с замером до/после.
Замеры: `cargo test --release -p parity --test self_session -- --ignored --nocapture`
(строки `MEASURE …` печатаются тестом и никогда не ассертятся). Перед
каждым коммитом `cargo fmt`, `clippy --workspace --all-targets -D
warnings`, `cargo test --workspace` — зелёные.

- [x] **`d271317` — п.1, порядок от центра наружу:** `write_chunk_batch`
      сортирует отфильтрованный батч по квадрату дистанции от центра
      джоина, tie-break `(d2, chunk_x, chunk_z)`. Правило выведено из
      трейсов ванили на v2 (не из вики): все **11 flush'ей**
      `chunk-batch-vanilla.json` не имеют спадов d2, первый flush = ровно
      **3×3 вокруг (6,−2), центр первым**; `join-vanilla.v2runB.json`
      повторяет эти 9 байт-в-байт; 5 глобальных спадов — только на
      границах flush'ей (flush сортирует уже загруженное). Замер (289
      чанков): спады d2 **136 → 0**, первый чанк **(−2,−10) → (6,−2)**,
      центр под индексом **160 → 0**, первые 9: среднее d2 **97.8 → 1.33**
      (ваниль 1.33, 9/9 со стороны chebyshev ≤1).
- [x] **`da8d2ee` — п.2, общий кэш готового батча:**
      `TemplateWorld.ready: OnceLock<Arc<Vec<LevelChunkWithLight>>>` —
      первый читатель (прогрев) строит, каждый вход клонирует только
      указатель; контракт `ChunkSource::load_batch` → `Arc<…>`, отправка
      идёт по ссылкам (payload не копируется; глубокий клон `Vec` замерен
      и оказался **медленнее** бэйзлайна, поэтому `Arc`), `world batch
      built` пишется один раз. Без новых блокировок (Arc+OnceLock; в
      `DECISIONS.md` запись о запрете новых Mutex не потребовалась —
      их нет). Замер connect → чанк, второй вход (кэш): **6058–7747 /
      6275–8054 мс → 3015–3022 / 3222–3232 мс**; первый вход (кэш
      строится там же): **5624–6672 → 5130–5442 мс**.
- [x] **`e8376a5` — п.3, per-chunk `info!` → `debug!`:**
      `level_chunk_with_light sent` (x/z/размер) переехал на debug в
      `write_sub_batch`; сводки (`chunk batch sent`,
      `chunk_batch_finished sent`, `world batch built`, отчёт радиуса)
      остались на info. `self_session` ставит подписчика и печатает
      `MEASURE logs: …`. Замер за два входа: все info-строки
      **601 → 23**, из них per-chunk **578 → 0**.
- [x] **`4917d09` — п.4, view/simulation distance из `server.toml`:**
      поля `view_distance`/`simulation_distance` в `ServerConfig`
      (serde-default 10 — пара, которую parity-тест `login_fields`
      читает из настоящего ванильного джоина; валидация > 0, 3 новых
      конфиг-теста), `net::WorldInfo` несёт оба поля,
      `send_radius(client, server)` ограничивается уже `view_distance`
      сервера, `build_world` пробрасывает их в обеих ветках,
      `server.example.toml` описывает пару с SOURCE, скомпилированная
      константа `VIEW_DISTANCE` удалена. Gate зелёный, push = 0.
- [x] **`6d113a8` — п.5, пейсинг порций по данным трейса:** гипотеза
      «батч писался целиком в ветке loading → ответ клиента приходил
      после него» подтверждена трейсом `chunk-batch-vanilla.json`:
      **72 чанка, 11 порций `[9,8,8,3,8,8,2,6,8,4,8]`, ровно один ack
      `8.0` сразу после finished первой порции** и дальше ни одного,
      после ack ни одна порция не шире 8 (= `ceil 8.0`), первая до ack
      = 9 (3×3 вокруг центра). Реализация: новый `play/batch.rs` —
      `PendingBatch` c состояниями First → AwaitPace (guard **1 с**,
      наш, не из протокола — тихий бот иначе ждал бы вечно) → Pace
      (50 мс); первая порция `FIRST_FLUSH = 9`, по ack — немедленная
      следующая (`on_paced`), хвост по `ceil(chunks_per_tick)`; в
      `mod.rs` ветка `select!` `wait_turn(..) if pending.is_some()`.
      Доказательный юнит-тест
      `the_first_flush_is_nine_then_the_vanilla_paced_tail` играет
      `[9, 8, 3]` за ack 8.0 на реальном `play_state`. Замер
      `self_session` (289 чанков, бот ack не шлёт): порции **1 →
      `[9, 280]`** на вход; MEASURE `burst framing` (ANSI-коды полей в
      подписчике отключены — ими рвался подсчёт ключей). Запись о
      правиле — `docs/DECISIONS.md`, «Часть A2, п.5: пейсинг батча».

Ранее в A2: п.5 (`4289650` тесты, `248bcf1` fix fluid-заголовка,
`2c998ba` docs) — остатки chunk-diff объяснены в `docs/DECISIONS.md`.

## Часть M4.0 (а), раунд 2: walk-трейс, П1-П3, ЧАСТЬ III (выполнено)

Сделано только в тестовом инструменте (`crates/parity`), код сервера не
менялся. Полный контекст и факты: `docs/ROUND2-CONTEXT.md`.

- Фикс часов трейса `b74ac44` (`ms()` возвращал секунды), дозапись
  документации `070783e`; п.4 (спектор-assert вместо execute-проб) `b4d151c`,
  п.5 (критерий мира `chunk_status.py compare` + зона) `3ec8b97`.
- Прогоны П1-П3 на исправленных часах (`walk-run-2-2026-11-09.log`):
  П1 чист (10 004 мс, 428 чанков, 106 порций, режим ok, мир ok);
  П2 кик ванильи `Invalid move player packet received` после 30 ходов;
  П3 по правилу остановки не запускался. Ответ 173/441 — ранний обрыв
  старого прогона, со 10 с приходит 428 и поток продолжается.
- ЧАСТЬ III (замеры входа): `WalkMode::Preload` + `src/preload.rs` (`d53bf03`),
  оркестратор `tests/preload_measure.rs` (`6b91786`); четыре серии ×3
  прогона, все 12 — `Completed` по 441 чанк. Отчёт с таблицами
  min/median/max: `E:\MineRust-data\traces\preload-report.md` + 14 JSONL.
- Обнаружено в трейсе: серии «наш сервер» содержат ~3000 мс
  ожидания самого бота (`PROBE_WAIT` в `steps.rs` — наш сервер не шлёт
  `select_known_packs`); у vanilla этого нет. Сжатие выключено с обеих
  сторон (vanilla threshold −1, наш `SetCompression` не отправляет).
- Выводов быстрее/медленнее НЕ сделано — различия (сжатие, свет,
  догенерация, кэш payload, предзагрузка спавна у vanilla) остаются
  гипотезами финального отчёта.

## Часть M4.0 (а), раунд 3: кик П2, перемер без дефекта бота, разбор payload (в работе)

Инструмент `crates/parity`, код сервера не менялся.

- I.1/I.2 (`bc72232`, `4da8e53`): `move_player_pos` бота - 25 байт, 3 double BE
  + байт флагов 0x00, значения конечные, у спавна, шаг 0.2; формат совпадает с
  живым клиентом. Дефекта кодирования нет, по правилу владельца I.3 (П2/П3)
  не запускался, параметры не подбирались. Факты:
  `E:\MineRust-data\traces\part1-move-bytes.md`.
- II.1 (`7bf19aa`): ожидание `select_known_packs` условное - `PROBE_WAIT` 3 с
  только для ванильного порта, `PROBE_WAIT_NO_PROBE` 100 мс иначе; ванильные
  серии не тронуты.
- II.2: только наши серии пересняты (`preload2-ours-*`), отчёт
  `E:\MineRust-data\traces\preload-report-ours-2.md`. Cold: int->441-й чанк
  8609.6/8613.3/8718.2 мс, warm 8585.0/8610.7/9477.8 (было 12047/13034/13434
  и 13865/13916/14410 со старым 3-с ожиданием).
- II.3: разбор payload (`chunk_breakdown.py`, зеркало `chunk_payload.rs`,
  самопроверка по длине). Гипотеза владельца подтверждена: разница join
  17.97 MB на 441 чанк = 19.90 лишних массивов света по 2048 B; наш сервер
  шлёт 24 sky-массива в каждом чанке, ваниль - только непустые (2..7).
  Heightmaps и section blob совпадают. Разбор:
  `E:\MineRust-data\traces\ii3-payload-breakdown.md`.
- III (float `chunks_per_tick`): ваниль warm и наш warm, pace
  {4.6715, 15, 50} x3, все 18 — `Completed` по 441. Число порций:
  ваниль 108/29/12, наш 87/29/9. Время до 441-го (медиана): ваниль
  5921.6/5671.1/5601.3 мс, наш 8545.5/8537.3/8660.1 мс — float задаёт
  размер порций, но время входа меняет слабо. Отчёт
  `E:\MineRust-data\traces\preload-report-float.md`.

## Часть M4.0 (а), раунд 4: ЧАСТЬ I — kick-эксперименты и правило тик/ход (выполнено)

Инструмент `crates/parity`, код сервера не менялся. Полные данные —
`E:\MineRust-data\traces\part1-report.md` (в репозиторий не кладётся).

- I.0: бот старых прогонов ack'ал телепорт (`t=1157.5`), затем ходил
  `t=1186.1` — **до** `player_loaded` (`t=2160.6`); живой клиент ack
  (seq 52), тикал, `player_loaded` (seq 91) и только потом ход seq 92
  (`player_loaded` + 22.800 мс).
- Э1 (первый ход ждёт `player_loaded`, дальше как старый П2): кик после
  2 ходов → гипотеза «ход до загрузки» опровергнута.
- Правило из трейсов: каждый кик шёл сразу после хода, у которого с
  предыдущего хода **ноль** `client_tick_end`. Ход обязан идти после ≥1
  тика; для П2/П3 включён `tick_coupled: true` (ровно один ход сразу
  после тика). Э1/Э2 оставлены несцепленными намеренно.
- Э2 (ровно три `move_player_pos` живого клиента байт-в-байт + 5 с
  простоя) — **первый успех**, кика нет; по правилу владельца Э3 не
  запускался.
- П2 (302 хода, 23 150 мс, x 156.9) и П3 (427 ходов, 30 062 мс,
  (156.9, 28.9)) — оба `Completed`, 0 ходов с нулём тиков; режим доказан
  трейсом (`login.game_mode=3`, `abilities=0x07`); `player_position`
  после первого — 1.
- `forget_level_chunk` декодируется двумя **big-endian i32** (payload 8 Б),
  а не VarInt: `chunk_center` → `chunk_move(name, payload)` + юнит-тесты.
  Окно: П2 4 центра (6,−2)→(9,−2) + 21 forget; П3 4 центра (6,−2)→(9,1) +
  62 forget. Батчи: П2 121/484 чанка/23.71 МБ, П3 137/535/24.73 МБ.
- Мировой критерий STOP: полоса x 30..32 (П2) и z 22..24 (П3) — генерация
  ванили за пределами отправленного окна (боту эти чанки не слались,
  максимум x=20). Контроль (без бота / стоит 30 с / 50 шагов) — 0 новых.
  Геометрия = квадрат радиуса 23 вокруг последнего чанка бота (**гипотеза**).
  Решение владельца: считать ли такие чанки нарушением.

## Часть M4.0 (а), раунд 4: ЧАСТЬ II — не ограничен ли замер ботом (выполнено)

Инструмент `crates/parity`, код сервера не менялся. Отчёт —
`E:\MineRust-data\traces\part2-report.md`; прогон `part2-run.log`.

- Добавлен режим бота «только метрики»: без JSONL и без base64 во время
  прогона (счётчики, размеры полезной нагрузки и монотонные метки
  пакетов в памяти), один JSON пишется уже после выхода бота. Сокет
  теперь читается через `BufReader` в обоих режимах. Складывание фактов
  общее (`PreloadMetrics::record`), поэтому JSONL и метрики сравнимы по
  построению; `RunStats.socket_read_ms` копит время внутри `recv_packet`.
- Тёплая серия (один живой процесс на сторону, 4.6715, цель 441): наш
  сервер `metrics` медиана ~5.48 с против `JSONL` ~10.78 с; ваниль ~5.57
  против ~6.03–6.56 с. Доля времени, которое бот **не** читал сокет:
  ~82–84 % в JSONL против ~18–27 % в metrics. Обе стороны в metrics
  сходятся к ~5.5 с. Вывод (**гипотеза, подтверждена**): наши старые
  числа входа ограничены ботом (base64/JSONL), а не сервером.
- Старые числа входа помечены «замер ограничен ботом (гипотеза)», не
  удалены.

## Часть M4.0 (а), раунд 4: ЧАСТЬ III — шаблон мира v3 (выполнен)

Инструмент `crates/parity/tests/build_template_v3.rs`, код сервера не
менялся. Отчёт — `E:\MineRust-data\traces\part3-report.md`, прогон
`part3-run.log`, мир — `E:\MineRust-data\world-template-v3` (в репе только
скрипт и описание).

- Лимит `forceload add` на команду измерен опытом: **ровно 256 чанков**
  (257 → «Too many chunks (maximum 256, but specified 257)»), стороны
  8192…512 отклонены.
- Против хрупкости прогона: ватчдог гасится на время сборки через
  `max-tick-time=-1` с охранником `Watchdog` (Drop возвращает исходное;
  после убитого вручную запуска свойство восстановлено вручную, сейчас
  `60000`) и одна `forceload`-команда на одну синхронизацию (`say <токен>`,
  пауза 2 с). Без этого всплески forceload делали тик > 60 с и ватчдог
  убивал ваниль, дальше `stop` падал с broken pipe.
- Результат: цель 61×61 = 3721 чанка, центр (6,−2), x −24…36, z −32…28;
  16 команд × 244 (последняя 61), один круг `save-all flush` + рескан,
  `full` в цели = **3721/3721 за 156 с** (тест 284.64 с).
- Чистота: `forceload remove all` → «Unmarked all force loaded chunks»,
  `query empty: true` (ваниль пишет «No force loaded chunks were found» —
  старая проверка на «No chunks» давала ложный отказ, детект расширен).
- Размер: **61.12 MB** всего, 58.86 MB регион `.mca`; 7569 чанков в
  bbox x −37…49 / z −45…41 (87×87, кольцо 13 с каждой стороны — ваниль
  довородила вокруг forceload-зоны). Статусы: full 4225, structure_starts
  2528, biomes 280, terrain 272, initialize_light 264. Готовая зона
  (bbox `full`) — x −26…38, z −34…30 = **65×65, цель ±2** (4225 чанков).
- Сравнение v2 → v3: `only_in_A=[]` (v1/v2 не тронуты: 0 файлов за 2 ч),
  `only_in_B=5360` (1512 в цели, 3848 за её пределами), `status_diffs=1584`
  = 1248 `structure_starts→full` + 120 `biomes→full` + 112 `terrain→full` +
  **ровно 104 `initialize_light→full`** (ожидание владельца подтверждено).
  `heightmap_diffs=1615` (1584 по статусу + 31 со статусом, но другими
  картами высот, x −5…18, z −14…10), `light_presence_diffs=1614`.
  v2: 2209 чанков, 14.69 MB (регион 14.20 MB).

## Часть M4.0 (а), раунд 4: ЧАСТЬ IV — пишущий прокси (выполнено)

Инструмент `crates/parity`, код сервера не менялся. Файлы:
`crates/parity/src/proxy.rs` (модуль библиотеки, `run_proxy`/`ProxyConfig`/
`ProxyError`), `crates/parity/src/bin/mcproxy.rs` (бинарь
`--listen 127.0.0.1:25565 --out <dir>`), `crates/parity/tests/proxy_verify.rs`.

- Прокси слушает 25565 и ходит в ваниль 25566, пишет по файлу на соединение
  `proxy-conn-N.jsonl`: `seq`, `dir`, `state`, `name`, `id`, `b64`, плюс
  `read_ms`/`write_ms` — монотонные миллисекунды от старта соединения
  (`Instant::elapsed`) на момент чтения и записи пакета соответственно.
- Состояние определяется по трафику: `minecraft:intention` (serverbound) →
  Login/Status, `minecraft:login_acknowledged` (serverbound) →
  Configuration, `minecraft:finish_configuration` (clientbound) → Play.
- Запуск только при «непрозрачном» канале: отказ, если
  `network-compression-threshold` не отрицательный или `online-mode` включён;
  аварийный выход, если ваниль всё-таки шлёт `minecraft:login_compression`
  (сжатие) или `minecraft:hello` (шифрование).
- Проверка (`cargo test -p parity --test proxy_verify -- --ignored`): стоящий
  бот через прокси 10 с, без живого клиента. Итог — бот `Completed`,
  473 чанка, 10 030 мс; запись `proxy-conn-2.jsonl` = 14 287 строк,
  `seq` строго растёт, `read_ms` неубывает в каждом направлении,
  `write_ms >= read_ms` в каждой строке, диапазон меток 0.1…10 735 мс.
  Гистограмма `(dir, state)`: clientbound play 13 937, serverbound play 307,
  clientbound configuration 37, serverbound configuration 3,
  serverbound handshake 1, login 1, clientbound login 1. Непустой `b64`
  в 13 649 строках. Тронута только та копия мира, на которой работает
  ваниль (`Vanilla::start` каждый раз копирует свежий v2 в свой рабочий
  каталог); v1/v2/v3 не изменялись.

## Раунд M4.0 (а), раунд 5: инструменты, критерий окна r=10, пауза П3, ходьба П4/П5 на v3 (выполнено)

- **ЧАСТЬ 0** (`733caf9`): `tools/trace-analyze/analyze.py` — один анализатор,
  только агрегаты (`summary`/`window`/`gap`/`outside`), списки по `--list`,
  UTF-8. Регрессионные тесты (`crates/parity/tests/build_template_v3.rs`)
  только на то, что уже ломалось: `Watchdog` ставит `max-tick-time=-1` и
  возвращает файл байт-в-байт; детектор пустого `forceload query` ловит
  реальную строку ванили 26.3.
- **ЧАСТЬ 1** (`ea7e408`): старый критерий «нет новых чанков вне зоны»
  отменён. Новый: (а) внутри окна r=10 вокруг центра статус/содержимое не
  меняются; (б) новые чанки вне окна — ожидаемое поведение ванили, не
  нарушение. `tests/window_probe.rs` перепрогнал П2/П3 с сохранением мира
  (`world-window-p2|p3`). Вне окна +141/+279 чанков, все `structure_starts`,
  21–25 от центра; внутри 0 новых, `initialize_light→full` 21/41.
- **ЧАСТЬ 2** (`f61c452`): пауза 3682 мс в П3 — не генерация и не чтение;
  серверу нечего было отправлять, пока бот оставался в чанке (8,0); центр =
  текущий чанк бота.
- **ЧАСТЬ 3** (`6e271d6`): `WalkMode::WalkV3` + `in_zone_v3` (центр-чанк
  x∈[−14,26], z∈[−22,18]) и `Vanilla::start_from` (свежая копия v3 на прогон).
  П4/П5 (600 шагов) — Completed, spectator; центры П4 8 / П5 6,
  forget 138/178; внутри окна новых 0. `player_position` в каждом = **1** —
  это штатный телепорт при входе (`seq=54`, `t=1166.5` П4 / `1405.5` П5,
  `pos=(96.5,145.0,−31.5)`, `teleport_id=1`), **не откат**: правило «откат =
  стоп» ждёт второго.
- **ЧАСТЬ 4**: `ARCH-M4.md` §4 (трейс ходьбы есть; `forget_level_chunk` = 2
  big-endian i32; окно двигается за ботом), §6 (итоги раунда 5), пункт о
  критерии закрыт; инструкция `docs/LIVE-WALK.md` (живой клиент на копии v3 +
  прокси 25565, не запускалось).
- Серверные крейты (`server`, `net`, `world`, `protocol`) не менялись.

## Раунд M4.0 (а), раунд 6, ЧАСТЬ 1: форма окна по трейсу — 473, а не 441 (выполнено, анализ)

Только анализ: код сервера не менялся. В `tools/trace-analyze/analyze.py`
добавлена подкоманда `shape` (форма окна, карта сдвига, `player_position`),
добавлены регрессионные тесты `tools/trace-analyze/test_analyze.py`
(4 теста, `python tools/trace-analyze/test_analyze.py` → OK). Найден и
исправлен дефект декодирования `forget_level_chunk`.

- **Решено по трейсу** (`analyze.py shape`, П2/П4/П5): после джоина загрузка —
  **23×23 квадрат со срезанными углами** (полуширина 11), всего **473** чанка
  при `view_distance=10`; по |dx| (и |dz|): `0..6→23, 7→21, 8→19, 9→19,
  10→15, 11→13`, 473 = 529 − 56; набор совпадает с `|dx|2+|dz|2 ≤ 162`.
- **Решено по трейсу:** сдвиг центра на 1 чанк отдаёт ровно ведущий край,
  забывает зеркальный ведомый, loaded = 473. П4 (+x) **23/23**, П5
  (диагональ) **37/37** (23+23−9). Поэтому **23/37, а не 21/41**: 21 —
  ведущий край квадрата ±10, 41 — его угол.
- **Решено по трейсу:** первый сдвиг П4 (`t=6442.6`) — во время джоина
  (до 313 чанков `t=1278..6432`, в интервал ещё 160 `t=6446..8107`, затем
  473). 0 forgotten — окно не заполнилось. «473 вместо 441» — размер окна.
- **Решено по трейсу:** «откат 1» в таблицах П4/П5 — единственный штатный
  `player_position` при входе (`seq=54`, `t=1166.5`/`1405.5`,
  `pos=(96.5,145.0,−31.5)`, `teleport_id=1`), не откат.
- **[гипотеза]:** полуширина 11 = R+1 и правило среза углов одним R не
  доказаны. Форма окна теперь **измерена**; реализацию окна (M4.4) не меняли.
- Отражено в `docs/ARCH-M4.md` §4/§6 и D2, `docs/PROGRESS.md`.

## Раунд M4.1 (выполняется): игровой поток, регион, каналы

- **План** (`d5f65c5`): 10 строк в `ARCH-M4.md` §6 (модуль `net::game`,
  `Region`, тривиальный `Scheduler`, тик по абсолютному дедлайну, метрика
  длительности, kick за переполнение, реплей JSONL; порядок б→в→г→д).
- **ЧАСТЬ 2б:** новый модуль `net::game` — типы сообщений (`ToGame`:
  `Join`/`TeleportAck`/`Quit`; `ServerMessage::Send`; `PlayerConnection` с
  каналом наружу и флагом кика; `MoveSlot`), один `Region` + `Player`,
  тривиальный `Scheduler` («N регионов за такт»), `GameHandle`/`GameThread`
  и тик 20 TPS по дедлайну `start + i·50 мс` (`sleep` + `try_recv`, тик не
  ждёт соединений). Метрика «длительность тика» и дрейф — атомики (без
  локов). Соединение пока НЕ подключено.
- **Замер (release, `--ignored`):** 1000 idle-тиков — дрейф **0.0 мс**,
  jitter p50 0.38 / p99 1.03 / max 1.11 мс, длительность p50 0.001 / p99
  0.002 / max 0.043 мс. Критерии (дрейф < 5, p99 < 20) выполнены. Тест 1000
  тиков помечен `#[ignore]` (D6: в CI только smoke).
- **ЧАСТЬ 2в:** `net::serve` поднимает один игровой поток (detached, живёт,
  пока живы `GameHandle`). `play_state` получает `GameHandle`, берёт
  `entity_id` из счётчика (первый = 1 → пакеты входа не изменились), создаёт
  канал наружу + `KickFlag` + `MoveSlot` и шлёт `Join`; `ConfirmTeleportation`
  отправляет `TeleportAck`; пакеты движения кладут сырой payload в слот
  «последняя позиция» (без разбора — это M4.2); `PlayerGuard` шлёт `Quit` при
  любом выходе; `select!` получил ветку чтения `ServerMessage`. Аргументы
  `play_state` сгруппированы в `Session` (clippy 7 арг.). Вход (join) не
  менялся — тесты `entity_id == 1` и join-байты зелёные.
- **ЧАСТЬ 2г:** политика kick за переполнение исходящей очереди и
  перегруженный тик. `Region::deliver` кладёт `ServerMessage` через
  `try_send`: при `Full` поднимается `KickFlag::SendQueueOverflow`, игрок
  удаляется и возвращается `Kick` (тик не блокируется); при `Closed`
  ничего (соединения нет — не кик). Перегруженный тик: решение о сбросе
  дедлайна вынесено в чистую `overdue_by(now, deadline, config)` (лаг >
  `reset_after` периодов → дедлайн один раз «сейчас + период», пачки
  догоняющих тиков нет). Юнит-тесты: переполнение → kick + удаление +
  флаг; закрытая очередь → не kick; `overdue_by` на пороге и за порогом.
- **ЧАСТЬ 2д:** формат реплея §1 — JSONL `{tick, seq, region, src, kind,
  payload}`, новый модуль `net::game::replay` (`Replay<W: Write>` +
  `ReplayRecord`, `serde`). Тик пишет строку в момент применения события
  (`apply`/`apply_queued`), `seq` растёт внутри такта, включается
  переменной `MINERUST_REPLAY` (значение = путь к файлу; каждая строка
  сбрасывается на диск). `move` появится в M4.2, где движение
  применяется. Тесты: один сценарий (join→ack→quit) дважды даёт
  байт-в-байт одинаковый журнал; поля `tick/seq/kind/region/payload`
  проверены; выключенный журнал не пишет. `cargo test --workspace` —
  `net` 35+ тестов зелёные.
- **ЧАСТЬ 3 (запуск):** `tools\run-server.bat` (ASCII, `cd /d "%~dp0.."`,
  проверка `server.toml`; при отсутствии печатает поля `client_jar`,
  `packets_report`, `registries_report`, `blocks_report`,
  `vanilla_server_jar`, `vanilla_server_dir` и пример для
  `E:\MineRust-data\`, затем `pause`; иначе `cargo run -p server
  --release`) и `tools\run-server-vanilla.bat` (только `java -jar
  server.jar nogui` с рабочим каталогом `E:\MineRust-data\vanilla`,
  jar ищется в `vanilla\` затем `E:\MineRust-data\server.jar`; мира не
  создаёт). Сухой запуск: `run-server.bat` поднял сервер, `listening
  0.0.0.0:25565` за ~1 с, `WARN`/`ERROR` нет, шаблон мира не изменён
  (36 файлов, 15 402 135 Б, mtime прежний). Чек-лист живой проверки —
  `docs/LIVE-CHECK-M4.1.md` (не выполнялся), инструкция по батникам — в
  README.

## Раунд 7: остановка игрового потока + окно чанков (выполнено)

- **Остановка игрового потока (ЧАСТЬ 1, `c7bfa68`).** Поток был **detached**
  (`serve` дропал `GameThread`, сигнала остановки не было; поток кончался
  только при закрытии канала — то есть при живом сервере никогда). Добавлен
  `Arc<AtomicBool>`-флаг: `game_loop` читает его **раз за тик** и выходит
  (`stop requested, leaving the tick loop`); `GameThread::join_timeout(2 с)`
  ставит флаг и ждёт `is_finished()` (опрос 5 мс), `join()` (тестовый путь)
  флаг **не** ставит (иначе ломается тест `ticks_limit`). `net::serve` —
  `tokio::select!` между accept-циклом и `shutdown_signal()` (Ctrl+C везде,
  на Windows ещё Ctrl+Break и закрытие окна через `tokio::signal::windows`),
  после выхода `join_timeout`. Фича tokio `signal`. Тесты на остановку:
  `the_thread_stops_on_request_within_the_timeout`,
  `join_timeout_reports_a_thread_that_did_not_stop`.
- **Живая проверка (выполнено).** Процессу (создан с
  `CREATE_NEW_PROCESS_GROUP`) послан `CTRL_BREAK_EVENT`: лог
  `shutdown signal received` → `stop requested, leaving the tick loop
  (ticks=11)` → `game thread stopped`, выход с кодом 0 за <1 с. Ctrl+C
  адресно группе процессов на Windows не послать (тот же
  `SetConsoleCtrlHandler`), закрытие окна — `CTRL_CLOSE_EVENT`.
- **Окно чанков (ЧАСТЬ 2, только доки).** `docs/ARCH-M4.md` §4 и
  `docs/DECISIONS.md`: формула окна (`|dx|,|dz| ≤ R+1`, `dx²+dz² ≤ 162`)
  проверена **только** при `view_distance=10` (единственный трейс), для
  других значений — **гипотеза**; наш `send_radius` шлёт 21×21 = 441, ваниль
  473, расхождение закрывается в **M4.4**.
- Код M4.1 не менялся; **M4.2 не начинался** (ждём «M4.2 можно»).

## Раунд 8: позиция игрока (M4.2, выполнено)

- **Соединение (`ca8d310`).** Пакеты движения больше не пишут
  `trace!("movement received, ignored")`: классифицируются по runtime-id
  (`MoveKind { Pos, PosRot, Rot, StatusOnly }`) и сырой payload кладётся
  в `MoveSlot` (`MoveState { kind, payload }`, было `{ packet_id, ... }`)
  приватной `store_move()` в `play/mod.rs`. Движение в сетевой задаче не
  интерпретируется.
- **Разбор в тике.** Новый модуль `net::game::movement`: `MoveUpdate`
  (опциональные оси + `flags`) + `MoveState::decode` (3×f64+1,
  3×f64+2×f32+1, 2×f32+1, 1; затем `ensure_empty`). Неполный/лишний
  payload — ошибка (`warn` + дроп), не догадка.
- **Флаги движения (фикс `a994d23`).** Последний байт move-пакетов — набор
  флагов, а не `bool`: живой трейс `live-session.json` несёт `0x00..0x03`
  на `move_player_pos`/`_pos_rot` (и `0x01`/`0x03` на `_rot`, `0x01` на
  `status_only`). `read_bool` (`byte != 0`) `0x02`/`0x03` **не** отвергал,
  но терял бит 0 — пакет `0x02` ошибочно считался «на земле». Теперь байт
  читается как `flags` (сырой, `MoveUpdate::flags`), `on_ground` = бит 0,
  `0x00..0x03` не отбрасываются; в реплее появилось поле `flags`. Значение
  бита 1 по нашим данным **не установлено** — не именуется. Тест на
  `0x00..=0x03` для pos и pos_rot.
- **Применение (`net::game::region`).** `Player` держит `PlayerPos` (вход —
  центр блока спавна `spawn + 0.5`); `Region::tick` забирает новейший
  `MoveState`, применяет его **только** после `teleport_confirmed` (иначе
  дропает) и накладывает оси через `Player::apply_move` — без проверок
  движения (осознанная дыра M4, `DECISIONS.md`). `Region::tick`/
  `Scheduler::tick` возвращают `TickOutcome { moves: Vec<AppliedMove>,
  kicks: Vec<Kick> }` (было `Vec<Kick>`).
- **Реплей.** `record_moves()` (общая для живого цикла и теста) пишет
  строку `kind = "move"` с полями `x/y/z/yaw/pitch/on_ground` после
  событий такта (`seq` продолжает счётчик `apply_queued`).
- **Тесты (`net` 20 passed, 1 ignored).** Декод каждого вида (5), игнор до
  `TeleportAck`, применение после ack, rotation не двигает позицию, слот
  хранит только новейший пакет, replay-детерминизм с `move` (сценарий
  дважды — байт-в-байт). `cargo test --workspace` зелёный.
- **Живая проверка (автоподмена).** Бот `parity` по нашему серверу не
  ходит: `enforce_mode` (все режимы кроме `Preload`) требует spectator
  (`login game_mode 0; expected 3`), а `Preload` не двигается. Поэтому e2e
  проверен одноразовым smoke (в репо не попал): in-process сервер +
  ручной `parity::Client` (join → `accept_teleportation` id 1 →
  20×`move_player_pos` шагом 0.2 от спавна 96.5,145,−31.5). Реплей: 21
  строка (join, teleport_ack, 19 move), x растёт 96.7→100.3 по одному move
  за тик — без скачков. Ручной чек-лист F3 — `docs/LIVE-CHECK-M4.2.md`.
- **Отложено.** `set_chunk_cache_center`/перенос центра кэша — **M4.4**
  (стриминг), сейчас не делаем (лишний пакет и риск для join). Телепорт
  тик уже подтверждает (`TeleportAck`). **M4.3 не начинался.**

## Раунд 9: M4.3 часть A — опоздания тика и пул регионов (выполнено, кроме B)

- **A1.1 (`overdue_by`).** Сброс дедлайна в `game_loop` происходит только
  при отставании > `period · reset_after = 250 мс`; живой drift 261 мс —
  это ≈211 мс отставания, сброса **не было**. 261→1.8 мс уменьшил догон:
  `sleep_until` возвращался сразу, `deadline += 50 мс`, `now` рос лишь на
  время работы — зазор падал на ~период за тик.
- **A1.2 журнал (`c45c3b9`).** `LATE_TICK_MS = 20` (`metrics.rs`),
  `TickMetrics.late_ticks`/`late_max_ms`, одна строка `warn` на тик
  позже порога (tick, late_ms, uptime_ms, players; признак батча опущен);
  `Scheduler::player_count()`.
- **A1.3 эксперименты (Э1/Э2 по 3 прогона, in-process сервер + ручной
  `parity::Client`, join → `accept_teleportation` → 289 чанков → 10 с
  простоя, release). `E:\MineRust-data\traces\m43-jitter.md`.** Э1
  (реплей выкл): late>20 мс 0/0/0, jitter max min/med/max 1.052/1.265/1.337,
  p99 1.012/1.015/1.040; Э2 (реплей вкл): late 0/0/0, max
  1.123/1.188/1.266, p99 1.011/1.027/1.043. **Живое опоздание 261 мс не
  воспроизвелось**; Э3 не запускался (условие «есть опоздания в Э1» не
  выполнено), хук приоритета не добавлялся.
- **A1.4 реплей.** Запись синхронна в тике (`Replay::record` → `flush` на
  строку). Данных, что она даёт выбросы, нет (Э2 vs Э1 — опозданий нет,
  `duration_max` +≈0.1 мс); причина выброса 32.7 мс **не установлена**,
  запись не менялась.
- **A1.5 вывод.** Источник живого 261 мс — **не установлено**; отличие
  живого лога — живой клиент (рендер/GPU), реальная сеть, реплей. В M4.6
  уходит гипотеза «клиент + сеть + приоритет потока тика».
- **A2 пул (механизм).** `net::game::pool`: `K` `std::thread`
  («minecraft-region-i»), регион передаётся по владению (mpsc), воркер
  тикает и возвращает; новых `Mutex`/`RwLock` в горячем пути нет;
  `Drop for Pool` закрывает каналы и ждёт воркеров ≤2 с. `Scheduler`
  собирает ровно отправленное число результатов (барьер) и merge-ит по
  `region_id`; при ≤1 регионе — inline (M4.1-путь), поэтому живое
  поведение не изменилось. `Region::tick` обходит игроков в порядке id
  (детерминизм merge).
- **A2 параметр.** `GameConfig.workers` (default 0), `ServerConfig.workers`
  в `server.toml` (default `ядра−2`, минимум 1; `0` = без пула),
  `net::serve_with(GameConfig, ...)`; `net::serve` делегирует с
  `GameConfig::default()`, `server::run` передаёт `config.workers`.
- **A2 тесты (а–г).** (а) K=0/1/4, 8 регионов — равный `TickOutcome`;
  (б) merge не зависит от порядка завершения (задержка 50 мс в регионе
  0); (в) пул останавливается < 2 с; (г) 1000 пустых тактов K=4 (4
  региона): drift 0.597 мс, jitter p50 0.487 / p99 1.030 / max 1.479 мс;
  плюс тик игрового цикла с пулом. Inline vs передача на 1 регионе
  (20000 тактов, release): inline p50 0.100 / p99 0.100 мкс, передача
  p50 0.700 / p99 1.900 мкс → решение inline (**гипотеза** до M4.6).
- **Отложено было.** Часть B M4.3 (`Transfer`, второй регион, реплей-тест
  исходящих) — сделана в раунде 10 ниже.

## Раунд 10: M4.3 часть B — карта регионов, `Transfer`, транскрипт (выполнено)

- **B1 карта регионов и `Transfer` (`92f4219`, `87a3c81`).**
  `net::game::grid::RegionGrid` (`region_side` чанков на тайл, деление с
  округлением вниз, тайл упакован в `i32`); `GameConfig.region_side` /
  `ServerConfig.region_side` (default **0 = нет разбиения**, весь мир =
  регион 0) и `server.example.toml`. `Region.players` — `BTreeMap`
  (детерминизм обхода); `Transfer { player, from, to, seq }`,
  `AppliedMove.seq`, `Kick.region`. `Scheduler`: индекс
  `player_id → region_id`, маршрутизация `Join`/`TeleportAck`/`Quit`,
  применение `Transfer` **после барьера** в порядке `(from, seq)` с
  переносом `Player` по владению, ленивое создание регионов по id.
  `TickOutcome::transcript` (порядок `(region, seq, kind)`); строка реплея
  `kind="transfer"` `{player, from, to}`. Живой сервер при `region_side=0`
  не меняется (тик inline, пул простаивает).
- **B2 сценарии (`86d9f75`).** `net::game::region_tests`, без сокетов,
  сторона 2 чанка, K=0/1/4: (а) переход границы — движение применяется в
  источнике, `Transfer` в том же такте, позиция непрерывна, дальше тикает
  новый регион; (б) четыре игрока пересекают в одном такте в обе стороны и
  в разные регионы — порядок `(from, seq)`; (в) `Quit` идёт к текущему
  владельцу (до/после `Transfer`); (г) неподтверждённый не переносится,
  подтверждение переживает `Transfer`; (д) пять игроков × десять тактов —
  на каждом такте каждый игрок ровно в одном регионе (без потерь и дублей).
- **B3 транскрипт и реплей (`b6fd13b`).** Канонический «исходящий» = результаты
  такта в порядке `(tick, region, seq)` (настоящие пакеты — M4.4).
  `net::game::replay_tests` (+ общие фикстуры в `test_support`): (а) один
  сценарий дважды в отдельных планировщиках — байт-в-байт одинаковые
  транскрипт и журнал; (б) транскрипт одинаков при K=0/1/4/8; (в) плейбек
  журнала (только входные строки `join`/`move`/`teleport_ack`/`quit` к
  свежему планировщику) воспроизводит живой транскрипт при K=0/1/4 **без
  смены формата** (записанный `move` несёт абсолютную позицию, `pos_rot`
  идемпотентен).
- **10.11.2026, живая проверка владельца.** Флаги движения: 499 move'ов,
  `flags=2` — 120, `flags=3` — 66 (упор в стену и прыжок), это только
  **корреляция**, бит 1 **не установлен**. Опоздания тика: view distance 32,
  наш send radius 10, `late_ticks = 0`, jitter p99 ≈ 1.1 мс; опоздание
  288 мс от 09.11 **не повторилось**, причина **не установлена**.
- **Тесты/gate.** `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` — зелёные: net 67 passed / 4 ignored, server 30.
- **Гипотеза до M4.6.** Значение `region_side` (D3) и выигрыш параллелизма
  на реальном шаблоне (он влезает в один регион — риск б). Живой чек-лист —
  `docs/LIVE-CHECK-M4.3B.md`.

## Раунд 11: M4.4 часть A — окно входа, индекс (выполнено, СТОП — часть B ждёт «M4.4 B можно»)

- **ЧАСТЬ 0 (замер, `c758691`, `5508a22`).** `crates/parity/tests/join_window.rs`
  (`WalkMode::JoinWindow`/`Stop::WindowIdle`, `analyze.py shape-join`): ваниль
  на свежей копии v2 при R=6/8/10/12. Форма — квадрат полуширины `R+1` со
  срезанными кругом `dx2+dz2 ≤ K` углами: R=6 → 213/h=7/K=74, R=8 → 329/9/113,
  R=10 (контроль) → **473**/11/**162**, R=12 → 637/13/208. Все симметричны,
  v2 ничего не теряет, шаблон не изменён (побайтно). `K(R)` формулой **не
  установлена** → форма из трейса — только R=10. Таблица — `ARCH-M4.md` §4.
- **ЧАСТЬ 1 (код, `0b5ba9d`).** `net::chunks::window` (473 при R=10, иначе
  квадрат `(2R+1)2`; порядок `(dx2+dz2, x, z)`), `net::chunks::ReadyChunks`
  (payload'ы + индекс `(x,z)→позиция`, один раз на прогреве, O(1), без
  блокировок), `ChunkSource::load_batch` → `Arc<ReadyChunks>`, `PendingBatch::new`
  строит окно и резолвит индекс, `server::TemplateWorld.ready` несёт индекс.
  `DECISIONS.md` — «гипотеза, не проверено по трейсу» для R≠10.
- **ЧАСТЬ 2 (проверено, этот коммит).** Наш in-process сервер +
  `crates/parity/tests/join_window_ours.rs`: R=10 → **473** (0 вне окна, 0 нет
  в v2, порядок = `net::window`), R=8 → **289**. Множество R=10 совпало с
  ванильным контролем **473/473** (`analyze.py shape-join --list`); payload'ы
  не сверялись. Живой чек-лист — `docs/LIVE-CHECK-M4.4A.md`; `ARCH-M4.md` §4/§6.
- **Не начато.** Часть B (стриминг при ходьбе: догрузка/`forget`/`StreamChunks`)
  — только по «M4.4 B можно».

## Раунд 12: M4.4 часть B — стриминг при ходьбе (выполнено)

- **ШАГ 0 (гипотеза, `24b5215`).** `K(R)=h2+3+isqrt(12h2)`, `h=R+1`, для
  `R∈6..=12`; вне диапазона квадрат `(2R+1)2`. Совпала с четырьмя
  измеренными K-интервалами (213/329/473/637). **[гипотеза]**, вне `6..12` не
  проверена. `net::chunks`, `ARCH-M4 §4`, `DECISIONS.md`.
- **ШАГ 1 (решено по трейсу, `ab30436`).** `forget_level_chunk` id 38 (0x26);
  порядок `set_chunk_cache_center`(new) → все `forget` → `chunk_batch_start` →
  `chunk` → `chunk_batch_finished`; забытия по возрастанию `(x,z)` (6/6 П4,
  5/5 П5). Кодек + тест на реальных payload П4.
- **ШАГ 2 (решено тестом, `10e9284`).** `TickOutcome.streams` +
  `StreamChunks { region, player, seq, center, add, remove }`; `add =
  window(new)−window(old)` в порядке нового центра, `remove` по возрастанию
  `(x,z)`; доставка `Scheduler::deliver` (`try_send`, переполнение = кик).
- **ШАГ 3 (решено тестом, `8f52bab`).** `PendingBatch` — очередь на всю
  сессию (`queue`/`sent`/`center`/`radius`/`not_cached`/`opened`,
  `Stage::Parked`), `move_window`, инвариант `within_window`
  (`sent ∪ queue ⊆ window(center)`), `write_window_move` (центр → forget →
  догрузка). Логика джоина не менялась. `join_window_ours.rs` R=8 289→329.
- **ШАГ 4 (решено по трейсу, `93ea00d` + этот раунд).** Порядок слов
  `forget_level_chunk` исправлен на packed `ChunkPos` (z старшим, x младшим;
  доказано 5/5 на П4). Новая ignored-обвязка `window_walk_ours.rs`: прогрев
  в бюджете (release **13.5 с**, ready **4481**, кэш **361 МБ** — лимиты
  20 с / 1.5 ГБ) + ходьба П4/П5 на v3 в in-process сервере. Центры **8/8** и
  **6/6** совпали с ванилью; множества/порядок забытий совпадают (у нас —
  полное уходящее множество; +7 на маршрут — темп джоина); инвариант окна
  выполнен (0 вне окна, живое множество == окно 473). Тик: p50 0.017-0.028 /
  p99 0.14-0.35 / max 0.28-4.28 мс, дрейф < 0.6 мс. Тест-обвязка греет мир
  до старта (как `server::run`); без прогрева первый вход читает мир инлайн
  ~13 с и ранние сдвиги окна отбрасываются — артефакт, не дефект.
- **Открыто (гипотеза, не проверено).** Дистанция **100+ чанков** на v3
  недостижима (пояс 65×65, запас ≈42 чанка) → **M4.6, синтетический
  провайдер**.
- **Файлы серверного крейта:** `net/src/chunks.rs`, `net/src/game/{region,
  messages,scheduler,mod}.rs`, `net/src/connection/play/{mod,batch}.rs`,
  `net/src/{connection,lib}.rs`; `protocol/src/packets/play.rs` (только кодек);
  `parity/src/walk.rs` (бот), `parity/tests/window_walk_ours.rs`.

## Что дальше

- **M4 — план показан владельцу, код НЕ пишется до «план принят».**
  Состав плана по требованию владельца: тик-цикл 20 TPS, учёт позиции
  игрока по пакетам движения, стриминг чанков при перемещении, первый шаг
  к регионам и многопоточному планировщику. Движок света не трогаем.
- Закрытая 6-пунктовка: (1) отчёт `target/parity-report/live-vs-vanilla.md`
  показан владельцу; (2) явная матрица 9 чанков × 6 категорий в
  `target/parity-report/chunk-diff.md`; (3) причина block entities (7,−2)
  и запись о картах высот — `docs/DECISIONS.md`; (4) guard `ServerGuard`
  (Drop: abort ин-процесс сервера + taskkill) уже в `join_slice`/
  `self_session`, `Vanilla` гасит java своим Drop; (5) `SOURCES.md`
  (живой клиент подтверждён 08.11.2026) и этот файл; (6) план M4.

- M3: чтение ванильного мира, отправка чанков — условие успеха: игрок видит
  мир из ванильного сохранения. Каркас `parity` готов (срезы status/join
  прогнаны, дифф-движок и отчёты работают); таблица `chunk-diff` уже
  сгенерирована и показана владельцу (решение по шаблону — `initialize_light`
  против `full` — за ним).
- M4+: сжатие (Set Compression), когда Registry Data/чанки перестанут
  влезать в лимит фрейма (решение — `docs/DECISIONS.md`).

## Известные проблемы / блокеры

- **[СНЯТ 08.11.2026] Блокер `seed` внутри compound `data`** закрыт
  коммитом `21c5d1d` (`fix(world): read the generation seed inside the data
  envelope`, fallback на корневой `seed` сохранён) + аудитом всех
  документов шаблона (`b9e89bf`, `crates/world/tests/template_docs.rs`) и
  снятием записи из `self_session` (`deea31c`). Сервер со `world_dir`
  стартует, живая проверка клиентом 26.3 прошла. Оставшаяся доказательная
  база блокера (для истории):
  - реальный файл `E:\MineRust-data\world-template\data\minecraft\world_gen_settings.dat`:
    корень `{DataVersion, data: {seed: 12345, bonus_chest,
    generate_structures, dimensions}}` — `seed` **в корне отсутствует**;
  - `level.dat`: корень `{Data: {...}}`, `Data.DataVersion = 5023`,
    `Data.spawn.pos = [96, 145, -32]` — читается нашим парсером ✓;
  - unit-тест `settings_nbt` синтетический (seed в корне) и не ловит
    расхождения с реальным файлом.
  Симптом был: `error: world document ... world_gen_settings.dat is
  malformed: no seed long at the root` → сервер с `world_dir` не стартовал.
  Фикс применён (`21c5d1d`: `root.data.seed` с fallback на корень, оба
  формата) — блокер закрыт.
- M2 закрывает play-состояние только join-последовательностью: чанков нет,
  движение игрока принимается, но не применяется (пустой мир, симуляция —
  M3+). Кик по неответу keep-alive реализован (30 с после неотвеченного
  keep-alive, `ConnectionError::KeepAliveTimeout`; само значение — вики,
  unverified).
- `hashed_seed` Login (play) теперь берётся из сохранения: формула сверена
  с трейсом ванили (`protocol::hashed_seed`, шаг 3.2), значение читается
  из `world_gen_settings.dat` шаблона (seed 12345, шаг 3.3e); спавн — из
  `level.dat` `[96,145,−32]`, его байты на проводе совпали с трейсом
  ванили (шаг g).
- Блокер палитры чанков снят: `E:\MineRust-data\reports\blocks.json`
  сгенерирован владельцем (`tools\generate-reports.ps1`) и прописан в
  `server.toml`; mapping имя+свойства → state id работает (сверено на
  всех 14 секциях шага 3.2).
- Флейк `join_slice` (остававшийся живым `target\debug\server.exe`)
  **закрыт guard'ом** (`284c731`): `ServerGuard` c `Drop` создаётся в
  начале теста до любых `expect` — при любом исходе (паника включительно)
  абортит in-process сервер и убивает `server.exe` через taskkill; у
  `Vanilla` свой `Drop` гасит java-процесс. То же дублировано в
  `self_session.rs`.
- Join-состав всё ещё расходится (вензель, чанки, 3748 vs 46 пакетов и
  `select_known_packs`) — это ожидание решений после живой проверки части 3
  (`docs/DECISIONS.md`), не забытые правки.
- Расхождения поля в поле по чанкам лежат в
  `target/parity-report/chunk-diff.md` (шаг 3.3f + явная матрица 9 чанков
  × 6 категорий): свет — 0/9 по всем маскам и массивам (до движка
  освещения), data-слова блочных палитр и заголовки non-air/fluid — по 8
  из 9, слова карт высот — 4 из 9, биомы — 9/9. **Причина блоков и block
  entities (7,−2: vanilla 2 vs у нас 0) найдена и записана в
  `docs/DECISIONS.md`**: 8 из 9 спавн-чанков шаблона лежат в статусе
  `initialize_light` и ваниль при загрузке догенерирует их до `full`
  (единственный `full`-чанк (6,−2) сходится 0 различий из 98 304 клеток);
  лечить — шаблоном, решение за владельцем. Карты высот — осознанное
  расхождение (ваниль пересчитывает при загрузке).
- Ид keep-alive у ванили = аптайм машины в мс, у нас — мс от старта
  сервера: origin разный по построению, побайтовое совпадение id не
  достигается и не преследуется.
- **Следствие v2 в `self_session`:** запись `target/parity/self-session.json`
  стала большой (~51 МБ, 289 чанков по ~90 КБ вместо 9) — `live_vs_vanilla`
  читает её дольше. Свет в NBT по-прежнему выборочный (подстановка `sky 15`
  сохранена), так что строка света в матрице чанков остаётся ✗ до движка
  освещения — это ожидание, а не регрессия (см. `docs/DECISIONS.md`).
- **Свет: временная подстановка sky 15 / block 0 для секций без света в
  NBT — светящиеся пещеры и резкие границы по подчанкам, лечится движком
  света (отдельный этап).** Подтверждено данными 09.11.2026 по чанку
  (4,0) шаблона v2 (`python target\parity\light_detail.py
  E:\MineRust-data\world-template 4,0`, статус `minecraft:full`), секции
  Y=0..4:
  | Секция | SkyLight в NBT | BlockLight в NBT | Что отправляем |
  |---|---|---|---|
  | Y=0 | нет | 2048 B, ненулевых 1 | sky = подстановка 0xFF×2048 (15), block = из NBT |
  | Y=1 | нет | 2048 B, ненулевых 115 | sky = подстановка 15, block = из NBT |
  | Y=2 | нет | 2048 B, ненулевых 7 | sky = подстановка 15, block = из NBT |
  | Y=3 | 2048 B, ненулевых 1920 | нет | sky = из NBT, block = 0 (empty_block_mask) |
  | Y=4 | 2048 B, ненулевых 2048 | нет | sky = из NBT, block = 0 |

  Граница подстановки (Y=2 → Y=3) — ровно граница подчанка, на которой
  наблюдается резкий перепад: ниже неё клиенту уходит синтезированный
  яркий sky, выше — реальные массивы из NBT. Правило подстановки —
  `crates/world/src/wire_light.rs` (§141, `docs/DECISIONS.md`); не
  исправлять до движка света.
- Детали протокола 26.3 не выдумываются: всё берётся из генератора данных
  и публичной документации протокола; недостающее помечать `TODO`.
  Теперь правило усилено: вики — только подсказка, непроверенное помечается
  `// SOURCE: wiki, unverified`, см. `AGENTS.md` и `docs/SOURCES.md`.
- Без `packets_report` в конфиге ID пакетов — дефолты из вики
  (неверифицированные); сервер при старте пишет warn. На машине владельца
  `packets_report` и `registries_report` прописаны в локальном
  `server.toml` — warn'ов нет; в репозитории только `server.example.toml`.
- `update_tags`: состав и значения совпадают с ванилью (parity-тест
  `update_tags_parity`), но **порядок** реестров в пакете и тегов внутри
  реестра у ванили — итерация внутренних Java-мапов сервера: в трейсе он
  зафиксирован для одного прогона, выводимого из данных правила нет.
  Решение владельца: порядок оставляем наш детерминированный — записано в
  `docs/DECISIONS.md`.
- **Живая проверка выполнена 08.11.2026**: настоящий клиент 26.3 вошёл на
  наш сервер — M1 (список серверов), M2 (вход) и 3.3 (первый батч чанков)
  подтверждены; трейс `target/parity/live-session.json`, сверка с ванилью —
  `target/parity-report/live-vs-vanilla.md`.
- `protocol_version`/`version_name` и путь к клиентскому jar из игры
  хранятся только в локальном gitignored `server.toml` владельца; в
  репозитории лежит `server.example.toml` с плейсхолдерами (0 / "unknown").

## M4.6 (crates/bench) — выполнено 12.11.2026

- Крейт `crates/bench`: capacity (W1/W2, K∈{0,2,6,10,12}), paced (дрейф/jitter),
  streaming (M4.4 «100+ чанков» — 125 сдвигов, инвариант окна 0/0, живое
  множество == окно 473; критерий M4.4 ЗАКРЫТ), load (N∈{10,50,100,200} ×
  K∈{2,6,10} × side{1,16}: 0 киков везде, join ~109 мс/бот), priority A/B.
- `Region::set_bench_workload(Duration)` (pub, default 0);
  `GameConfig.tick_thread_priority` (default off, Windows).
- Вердикты: D5 (K = available_parallelism−2 остаётся), D3 (region_side=0 до
  данных реального мира), D7 (приоритет off; late_ticks улучшаются под
  contention). Агрегаты: `docs/BENCH-M4.md`; сырые отчёты: `target/bench/`.
- Далее: M4.5 — трейс ванили «два бота» (C1–C3), только бот/инструменты,
  без серверного кода M4.5. ← этот шаг выполнен, см. следующий раздел.

## M4.5 (данные) — трейс ванили «два бота», 12.11.2026

- Драйвер `crates/parity/tests/two_bot_trace.rs` +
  `tests/common/{twobot,twobot_scenarios}.rs` (`88c61a7`): два бота под
  своими именами/offline-UUID, тик-сетка 50 мс, один прогон на сценарий.
  Разбор — `tools/trace-analyze/analyze.py entities` (`d8dd392`).
- Прогон: `cargo test -p parity --release --test two_bot_trace --
  --ignored --nocapture` = 197.48 с, С1/С2/С3 подряд, каждый на свежей
  копии `E:\MineRust-data\world-template`. Фон: CPU ~14 %, ~21 ГБ
  свободно, 12 логических / 6 физических ядер
  (`target/bench/m45-run-note.txt`).
- Итог: `violations=[]` везде, шаблон SHA-256 не изменился (0 файлов);
  оба бота spectator, спавн (96.5, 145.0, −31.5).
- Ключевые факты: С1 — A видит add ×165 / remove ×1 (только id 77,
  чистый уход B); С2 — B прошёл +26.6 блоков за 10 с (200 move-пакетов);
  С3 — потеря слежения на **160 блоках вдоль +x (= view_distance × 16,
  [гипотеза])**, B вернулся в точку входа (1602 move-пакета, наблюдение
  A 93.6 с / B 88.2 с). Ид B = 77, offline-UUID-гипотеза сработала
  во всех сценариях (add #58).
- Мир (перечислен, не останавливал): внутри ready-зоны новых чанков 0;
  вне зоны — С1: 0, С2: 47 в колонке x=30, С3: 464 в x 30..39 (ранняя
  стадия `structure_starts`, причина не установлена).
- Агрегаты: `docs/ARCH-M4.md` §6 («M4.5 (данные)»), `docs/DECISIONS.md`;
  сырые отчёты `E:\MineRust-data\traces\`, разборы `target/bench/`.
  Серверные крейты (`net`/`server`/`protocol`) не тронуты — бот-код и
  инструменты.
- Далее: решение владельца по объёму серверного M4.5 (видимость игроков)
  с учётом списка «чем трейс не покрыл» (ARCH-M4 §6).

## M4.5 (сервер, Шаг 0) — проверка выводов M4.6 (12.11.2026)

Владелец сверил числа M4.6 перед серверной частью M4.5: замеры и правки доклада.

- **Дефект бенча найден и исправлен (решено замером).** Раскладка
  `spread(64, 8, (−64, −64))` при `region_side = 64` клала все 64 игрока в
  **один** регион-тайл, поэтому `Scheduler::dispatch` всегда был inline и число
  K не влияло: 0.1а дал 1 регион, p50 ≈ 207 μs, tps ≈ 3430–3490 при K=0/1/2/4/10.
- Исправление: `bench::players::spread_over_tiles` (игрок в центре тайла, ровно
  `regions` регионов) + `Region::tick` крутит busy-loop **только** в регионе с
  игроками; раннер по времени `capacity::run_timed`; системный CPU-сэмплер
  `bench::cpu`; фазовый сброс метрик (`TickMetricsHandle::request_phase_reset`).
- **W2 после исправления** (S=50/200/1000 μs, регионов 4/16/64, 5 с прогрев +
  10 с, 3 повтора): медиана p50 ≈ `регионы × S / K`; speedup K=10 = 3.0/6.6/8.4
  (S=50), 3.6/7.5/8.8 (S=200), 3.9/7.9/9.0 (S=1000); потолок — число регионов и
  ~6 физических ядер; K=12 (SMT) надёжного выигрыша не даёт. Полная матрица:
  `target/bench/m45-w2-matrix.md`.
- **W1 (реальный дешёвый тик, 64 региона, 3 повтора)**: пул **дороже** (медиана
  p50 11 μs при K=0 → 162 μs при K=12); при 1 регионе (дефолт `region_side = 0`)
  тик идёт inline. `target/bench/m45-w1-matrix.md`.
- **Фазы (0.2)**: `bench phase`, N=100/K=6 и N=200/K=10, side=16, 3 повтора —
  фаза входа дёшева (p50 ≈ 0.01 мс, p99 ≤ 0.05 мс, late=0); высокий p99
  (4–14 мс) — в **установившейся ходьбе**. `target/bench/m45-phase-*.md`.
- Доклад: `docs/BENCH-M4.md` переписан (§0 дефект, §1 W1, §2 W2, §5.1 фазы,
  §6/§7 вердикты); `docs/DECISIONS.md` — D5/D3/D7; `docs/ARCH-M4.md` §5.
- Далее по плану M4.5-сервер: Шаг 1 — `analyze.py entities-detail` (фильтр по
  id B=77) и правила видимости из трейса → ARCH-M4 §6; Шаги 2–5 — кодеки,
  дизайн видимости, тесты, доклад/LIVE-CHECK.

## M4.5 (сервер, Шаг 3) — видимость и entity-поток (12.11.2026)

- **Видимость (Шаг 3, коммит `6c1a850`).** Новый `crates/net/src/game/entities.rs`:
  `visibility_pass` считает для каждого игрока невидимых ранее/пропавших
  («горизонтальный круг» радиуса `view_distance × 16`, без гистерезиса,
  симметрия через `min(r_a, r_b)`, себя не видит; наивный O(P²)). Новые типы
  `StreamEntities/EntitySpawn/EntityRemove/EntityMove/EntityMotion`. `Player`
  получил `visible`/`last_broadcast`; `TickOutcome.entities`; транскрипт —
  новый `kind: entity` (сорт 3), `kick` переехал на сорт 4; replay-journal не
  трогали (стриминг сущностей — транспортное состояние).
- `ServerMessage::StreamEntities` доставляется `deliver_entities` после
  `deliver_streams`. Пакеты собирает новый
  `crates/net/src/connection/play/entities.rs`: порядок спавна как в C1
  (player_info_update → add_entity → set_entity_data(11) → move_entity_pos(0x00)
  → set_entity_data(14) → update_attributes); движение Position/Rotation/Sync
  (позиция+поворот → pos_rot + rotate_head); скачок > ±8 блоков →
  entity_position_sync; удаление — `remove_entities` первым, а
  `player_info_remove` — только при уходе из мира и **после** него (порядок
  C1; уточнено в Шаге 4 (г), `546f538`). `PLAYER_INFO_GAME_MODE = 3` (в трейсе C1
  записано 3, не 0 — доверяем трейсу).
- Гейт: fmt / clippy `--all-targets` / test — зелёный (61 ok).

## M4.5 (сервер, Шаг 3.5) — замер видимости — СТОП (12.11.2026)

- Harness: `crates/bench/src/visibility.rs`, запуск
  `cargo run -p bench --release -- visibility` (capacity-движок, один регион,
  inline, без сети; игроки зарегистрированы до прогрева — фаза входа исключена;
  ходьба 0.25 блока/тик; 3 прогрева + 15 с на прогон, 3 повтора; P=64/200/512).
  Геометрии: **dense** — компактный квадрат с шагом 1 блок (все взаимно
  видимы, виден весь `P−1`); **uniform** — grid по 1000×1000 блоков
  (~14–16 видимых соседей при P=200).
- Результат (release, Ryzen 5 2600X, 6 ядер/12 потоков; p50/p99/max в мс):

  | geometry | P | p50 | p99 | max | ticks/s |
  |---|---|---|---|---|---|
  | dense | 64 | 1.39–1.42 | 2.41–2.58 | 5.6–10.9 | ~650 |
  | dense | 200 | 16.5–17.2 | 21.2–27.2 | 30.3–44.0 | ~58 |
  | dense | 512 | 144.5–149.1 | 154.5–203.8 | 180–204 | ~7 |
  | uniform | 64 | 0.51 | 1.31–1.36 | 2.4–4.1 | ~1600 |
  | uniform | 200 | 5.08–5.11 | 7.86–8.33 | 14.5–17.3 | ~184 |
  | uniform | 512 | 37.2–37.4 | 43.1–51.6 | 65.1–72.2 | ~26 |

- **Стоп-условие Шага 3.5 сработало:** установившийся p99 при (b) P=200 =
  **7.9–8.3 мс > 5 мс** (во всех 3 повторах). Дальше по плану — стоп, Шаг 4
  (тесты а–д) не начат.
- Сырой отчёт: `target/bench/m45-visibility-*.md`.

## M4.5 (сервер, Шаг 3.5) — пересдача по варианту 1 (12.11.2026)

Владелец выбрал вариант 1 (оптимизировать наивный проход) с условиями: сначала
измерить по фазам, оптимизировать то, что покажет таблица, доказать
эквивалентность, повторить замер. Выполнено 12.11.2026.

- **Инструментирование (`3e19476`).** Шов в `net`: `Scheduler::tick_core`
  (ходы, transfer, `StreamChunks`), `Scheduler::refresh_entities` (проход
  видимости), `tick` = их композиция; общий сборщик пакетов
  `net::entity_packets` (используют `connection/play/entities.rs` и bench);
  `BenchPlayer.rx` + `drain_outbound`. Режим
  `cargo run -p bench --release -- visibility-phases` меряет четыре фазы за такт:
  (а) видимость, (б) сборка пакетов для наблюдателей, (в) `try_send`, (г)
  остальной тик — p50/p99/max, 3 повтора, uniform P=200 + dense P=64/200/512.
- **Причина по таблице (не по оценке):** и при uniform P=200 (88 % времени p50),
  и при dense P=512 (74 %) доминирует **(а)** — проход видимости: наивный
  `visibility_pass` в горячем цикле делает 4–6 обращений к `BTreeMap` на каждое
  O(P²)-сравнение и перестраивает карты + `player.name().to_owned()` каждый тик,
  то есть фактически O(P² log P) с большими константами.
- **Оптимизация (`919fd75`).** Снимок позиций/радиусов/базлайнов в `Vec` один
  раз, один индекс `id → i`, попарный цикл и множественные операции
  (двухточечное слияние отсортированных новых/старых id) — по `Vec`, видимость
  игрока обновляется на месте; имена/UUID читаются только при спавне. Порядок
  игроков и пакетов — по возрастанию id.
- **Эквивалентность (тест `9b6e810`).** Прежняя наивная реализация оставлена
  эталоном в тестовом коде (`entities_reference.rs`); на 1100 случайных
  раскладках (P 2..64, разные радиусы, позиции точно на границе радиуса,
  отрицательные координаты, 3 раунда со случайными ходами) множества видимых
  пар и события новой и старой реализаций совпадают.
- **Повторный замер 3.5** (`visibility`): uniform P=200 p99 = **2.81–2.98 мс**
  (3/3) ≤ 5 мс — порог пройден. dense P=512 p99 упал 154.5–203.8 →
  14.1–14.5 мс; uniform P=512 → 6.06–6.18 мс; dense P=200 → 3.07–3.49 мс;
  dense/uniform P=64 → 0.87/0.75 мс.
- Фазы p50 (детали — `docs/BENCH-M4.md` §9): до — (а) 4.97 при uniform P=200;
  после — (а) 0.37, теперь p50 доминирует (б) (сборка пакетов, в проде на задаче
  соединения, не на тик-потоке). Тик-цена (а)+(г) = 0.39 мс p50.
- Гейт зелёный (fmt / clippy `--all-targets` / 61 ok). Сырые отчёты:
  `target/bench/m45-visibility-*.md`, `target/bench/m45-visibility-phases-*.md`.

## M4.5 (сервер, Шаг 4) — тесты (а)–(д) и доп. юнит-тесты (13.11.2026)

Набор владельца (дословно — `docs/ARCH-M4.md` §6). Коммиты: `483f3f4` (а, б),
`16b732f` (в), `546f538` (г), `cc45c99` (д), `5563291` (доп.). Гейт зелёный на
каждом пункте (fmt / clippy `--all-targets` / 14 сюит, net 105→109 ok).

- **(а)/(б) — пройдено** (`crates/net/src/game/step4_tests.rs`, in-process без
  сокетов). Сценарий 4 игроков: вход колонной, ходьба навстречу, поворот на
  месте (форма С4), уход из видимости, возврат, quit. Один транскрипт при
  `K = 0/1/4/8`, при повторе, при `region_side = 0` и `SMALL_SIDE = 2`; на
  каждом такте видимость симметрична и равна эталону `mutually_visible`, никто
  не потерян/продублирован, на малом тайле `transfers > 0`.
- **(в) — красный диагностический (ignored)**. Наш v3-сервер с настоящими
  сокетами, два бота, С1/С2/С4; фильтр по id B из UUID `add_entity`; нормализация
  только entity id / позиция по тику / game mode. Расхождения — списком (тип,
  смещение, наш байт, ванильный байт) в
  `E:\MineRust-data\traces\entity-stream-parity.md`. Типов: С1 11/8, С2 176/316,
  С4 113/198; причины — нет нулевого `move_entity_pos`, порядок
  `update_attributes`, нет `entity_position_sync` о игроке в С2, темп/байты
  `rotate_head`/`move_entity_rot`. Gate не задевает (ignored).
- **(г) — пройдено.** С3 на нашем сервере. Дистанцию мерим **геометрически** (от
  собственных часов B): наш remove 160.2–160.4 (ваниль 160.2 ± 0.2), re-add 159.8
  в интервале Шага 1.4 `[154.4, 160.2]`, максимум видимости 160.2. Наивные
  кросс-трейсовые числа анализатора (remove 154.6) смещены сдвигом часов двух
  настоящих клиентов (≈1.4 с). Правка `push_remove`: `remove_entities` первым,
  `player_info_remove` следом (порядок C1), тесты сначала красные.
- **(д) — «подтверждено отсутствие»** (`docs/ARCH-M4.md` §6). Входной
  `player_info_update` (36 Б) совпал с ванилью байт-в-байт (game mode 0);
  entry-parity `player_info.rs` зелёный, правка не нужна.
- **Доп. юнит-тесты — пройдено** (характеризующие). Граница радиуса на
  160/64/16/8/1 и с отрицательными координатами; себя не видит даже в толпе;
  кодирование Position/Rotation/Sync (скачок > 8 блоков → `entity_position_sync`);
  нет `teleport_entity`. net 105→109 ok.

## M4.5 (сервер, Шаг 5) — документация и живая проверка (13.11.2026)

- `docs/LIVE-CHECK-M4.5.md` — 8 шагов живой проверки **двумя** клиентами 26.3 на
  одной машине (вход, Tab с двумя именами, уход >160 блоков/возврат, порядок
  ухода из мира, fallback если второй экземпляр лаунчера не запускается).
- `docs/ARCH-M4.md` §6 — «Шаг 4 (а)–(г) и дополнительные юнит-тесты — результаты»
  и «Известные расхождения (в)»; §4 — видимость идёт в том же тике, что стриминг,
  окна не меняет; строка про M5+ (экипировка/скины/анимации/чат/атаки/мобы —
  вне M4.5).
- `docs/DECISIONS.md` — геометрический вывод (г), известные расхождения (в),
  «подтверждено отсутствие» (д).
- `docs/HANDOFF.md` — тесты а–д (2 строки) и следующий шаг.

## M4.5 (сервер, Шаг 6) — правила потока, сверка (в), замер (13.11.2026)

Коммиты: 6.1 `0b58e4c`, 6.2 `241cc7b`, 6.3 `83433f2`, 6.4 `2c6acec`.

- **6.1 — правила потока о движущемся игроке по трейсу** (`docs/ARCH-M4.md` §6
  «Шаг 6.1»): темп раз в 2 серверных тика; выбор пакета по смене против
  последней отправленной позы (pos+rot → `move_entity_pos_rot`+`rotate_head`,
  pos → `move_entity_pos`, rot → `move_entity_rot`+`rotate_head`, нет смены
  ≥60 тиков → нулевой `move_entity_pos`); байт угла = `floor(deg·256/360)` в f64;
  порядок `update_attributes` нестабилен между прогонами; триггер
  `entity_position_sync` — «не установлено»; инструменты
  `tools/trace-analyze/step6*.py`.
- **6.2 — перенос правил в код** (`crates/net`): счётчик
  `ticks_since_broadcast`, `UPDATE_INTERVAL=2`, `BROADCAST_PERIOD_TICKS=60`,
  `EntityMotion::ZeroDelta`, f64 floor-углы; наивный эталон повторяет каденс.
- **6.3 — сверка (в) после правки**: C1 сошёлся целиком (кроме порядка
  `update_attributes`); в C2/C4 четыре остаточных класса, **не выводимых из
  трейса** — (i) триггер `entity_position_sync`, (ii) байт `rotate_head`
  (противоречивые правила C2/C4), (iii) порядок `update_attributes`, (iv)
  джиттер сэмплинга ванили и период покоя C4. **Стоп на (i) и (ii)** по правилу
  владельца; данные — §6 «Шаг 6.3», `tools/trace-analyze/m45-step63-*.py`,
  отчёт `entity-stream-parity.md`.
- **6.4 — замер**: пакеты о движущемся игроке на наблюдателя на тик 0.83/0.97
  (наш C2/C4) против 0.74/0.80 (ваниль); p99 видимости uniform P=200 =
  2.689/2.749/3.491 мс ≤ 5 мс — порог пройден, стоп-правило 3.5 не сработало.
- **6.5 — документы**: `docs/ARCH-M4.md` §6 (подразделы 6.2/6.3/6.4 и итог),
  `docs/DECISIONS.md`, `docs/PROGRESS.md`, `docs/HANDOFF.md`;
  `docs/LIVE-CHECK-M4.5.md` — актуализация (net 112, пометка про известные
  остатки (i)/(ii) — не дефект живой проверки).

## M4.5 (сервер, Шаги 7.1–7.5) — режим игры, опоздания тика, документы (13.11.2026)

- **7.1 — причина живого «наблюдателя»** (код не менялся): в
  `entity_packets.rs` стояли константы из spectator-трейса C1 —
  `PLAYER_INFO_GAME_MODE = 3` и entity flags `0x20` (невидимость) во втором
  `set_entity_data`. Login при этом слал `game_mode: 0`. Нормализация game mode
  в (в) скрывала первое, совпадение с C1 — второе.
- **7.2 — ванильные C5/C6** (`crates/parity/tests/two_bot_gamemode.rs`,
  `PropertiesGuard` возвращает `server.properties`, `violations=[]`, 72 с на
  прогон): от режима зависят `player_info_update` offset 30 (`03`/`00`/`01`) и
  состав второго `set_entity_data` (`0x20` — только spectator, 14 Б; в C5/C6
  11 Б, как первый); одиннадцать байт базового снапшота одинаковы во всех
  режимах.
- **7.3 — режим как параметр сервера**: `GameConfig::default_game_mode`
  (default 0, только `0..=3`, иначе ошибка при старте), `PlayerJoin/Player/
  EntitySpawn/EntityMove.game_mode`, `send_join(..., game_mode, ...)`,
  `metadata_second(game_mode)`, заголовок нулевого `move_entity_pos` по режиму
  (третья зависимость, найдена при сверке). `PLAYER_INFO_GAME_MODE` удалена. В
  `server.toml`/`config.rs` параметра **нет** — решение владельца. Тест (в)
  больше не нормализует режим: C1/C2/C4 идут с 3, C5 — 0, C6 — 1. Итог:
  C1 — 1 дифф (iii), C5/C6 — по 6 строк (i + iii + сдвиг на один пакет),
  **ни одного байтового дифа по режиму**; C2/C4 — прежние (i)/(ii)/(iv).
  net 117 тестов ok, gate зелёный.
- **7.4 — опоздания тика**: `server::run` стартует тик **после** прогрева
  (`build_world` → `warm_world` → `bind` → `serve_with`), значит 7–9 с из живого
  лога — не прогрев. Замер `crates/parity/tests/tick_late_after_warmup.rs`
  (release, v3, 20 с без игроков): warm-up 8.34 с, 400 тиков, `late_ticks = 0`,
  `late_max_ms = 0.000`. Причина живых 103 мс — **не установлено** (нужен лог с
  `uptime_ms` и меткой входа).
- **7.5 — документы**: `docs/ARCH-M4.md` §6 (шаги 7.2/7.3/7.4),
  `docs/DECISIONS.md` (режим как параметр, ограничение spectator, снятая
  нормализация, открытый долг по self-записям), этот раздел, `docs/HANDOFF.md`,
  `docs/LIVE-CHECK-M4.5.md` (пометка про режим и `tick woke late`).

## M4.5 (сервер, Шаг 8) — разбор player_input/punch, состояние игрока, диагностика тика (14.11.2026)

Код написан по принятому плану Шага 8, четыре коммита (a–d). Кодировщики
пакетов наблюдателю **не писаны** — ждут ванильных трейсов S1–S4 (владелец
записывает их по `docs/RUNBOOK-STEP8-TRACES.md`, автоматизация —
`tools/record-step8.ps1`).

- **(a) `player_input`/`punch` и состояние игрока** (`e0ccda6`): serverbound
  `player_input` (0x2B, 1 байт), `player_command` (0x2A, три VarInt),
  `punch` (0x2E, пустой) — декодеры/кодеры + тесты по подтверждённым payload
  живого трейса. Состояние игрока: sneak/sprint (источник истины —
  `player_input`, биты 0x20/0x40 помечены как гипотеза «SOURCE: wiki,
  unverified»), счётчик swings (для будущего `animate`), `head_yaw` в
  `PlayerPos` (идёт за телом — отдельного источника не установлено).
  `MoveSlot` расширен: движение / ввод / взмахи — независимые записи одного
  слота (новых `Mutex` нет). Тик применяет ввод и взмахи даже без пакета
  движения. `rotate_head` теперь берёт байт из `head_yaw` (байты не
  менялись). Новый `state_tests.rs` (5 тестов).
- **(b) фазовая диагностика тика** (`28fe8bd`): warn «tick phases» при
  опоздании > 50 мс; явные определения: `late_ms = finished − deadline`
  (общее опоздалость = позднее пробуждение + медленная работа),
  `wake_late_ms = woke − deadline` (только пробуждение). Большой
  `wake_late_ms` при малых фазах → стопор снаружи (ОС/консоль/антивирус);
  большие фазы → работа тика, поле `*_us` называет фазу.
- **(c) non_blocking writer** (`server`): `tracing-appender`,
  `lossy(false)` (при полном буфере — backpressure, ничего не теряется),
  guard в `main` сливает очередь и ждёт writer до 1 с при выходе.
- **(d) документы и запись трейсов**: runbook S1–S4
  (`docs/RUNBOOK-STEP8-TRACES.md`), скрипт разбора
  (`tools/trace-analyze/m45-step80-scenarios.py`), автоматизация записи
  (`tools/record-step8.ps1 -Scenario S1..S4`).

Живая проверка после Шага 8 показала **те же дефекты A–D**, что и до
кодировщиков (так и должно быть: поза/взмах/плавание/поворот при входе ещё
не кодируются для наблюдателей). Это ожидаемое состояние до трейсов.

## Шаг 9: тестовые клиенты MC 26.3 + Fabric-мод + автосценарии (14.11.2026)

Автономный шаг: реальные клиенты игры с Fabric-модом `testclient` (JSON-сокет
управления на 127.0.0.1), автосценарии видимости A→B с сетевыми трейсами mcproxy.
Всё в `tools/testclient/`; Rust-код проекта не менялся.

- **Этап А (батники, `a51c156`)**: запуск ванильного клиента через `New-ClientLaunch.ps1`
  (natives в отдельный каталог, gameDir на клиента, quickPlay-вход). Приёмка: оба
  клиента на ванили через mcproxy 25565.
- **Этап Б1–Б5 (`4cdd72f`…`6ac4317`)**: Fabric-мод (Loom, JDK 25), JSON-сокет
  `{"id","cmd","args"}` → `{"id","ok",...}`; команды ввода (hold/tap/look/attack через
  `KeyMapping`), wait/disconnect/command/screenshot, `observe {player}` (поза/углы/режим
  наблюдаемого игрока). Приёмка трейсами: serverbound `player_input` (43), `punch` (46),
  `player_action` (41). `connect` заблокирован (решение владельца, гипотезы в README
  testclient). RCON-хелпер `Invoke-Rcon.ps1` (плюс фикс протокола RCON: ответ на
  команду — type 0, auth — type 2).
- **Этап В (`bb74858`)**: оркестратор `run-scenario.ps1` + `scenario-lib.ps1` +
  сценарии `scenarios/s1..s4.json`, `summary.json` в `E:\MineRust-data\autotests\<время>`.
  S1 (поза через перезапуск, сценарий владельца), S2 (дельты углов), S3 (взмах в воздух
  vs удержание на блоке, факт и порядок по seq), S4 (game mode). Приёмка: **14/14 PASS**
  на ванили (чистый прогон 2026-11-14_1644). Находки (по трейсам mcproxy, задокументированы
  в README testclient): поворот сам по себе не уходит серверу (нужен tap движения —
  `move_player_rot`/`player_input`); клиент не готов к командам ~3.5 с после join-строки
  (`player_loaded`); wire и NBT big-endian; playerdata 26.3 — `players/data/<uuid>.dat`
  с реверсом первых трёх групп UUID; `Write-Host` не попадает в pipeline.
- **Этап Г (`run-scenario.ps1 -Target ours`)**: прогоны S1–S4 против нашего
  Rust-сервера (server.exe за тем же mcproxy 25565→25566; join-gate по self-observe,
  RCON пропускается и пишется как пробел). Прогон 2026-11-14_1735: **S2 6/6 PASS**
  (дельты углов доходят до B, `move_entity_rot` ×3); остальное — честные пробелы
  возможностей сервера (не баги стенда): нет playerdata/`player_rotation` при входе
  (S1), `drain_swings` не подключён → взмахи невидимы (S3), `player_action` клиент
  не присылает вообще — копание не стартует (S3), нет RCON → смена game mode
  невозможна (S4). Таблица «байты → эффект» (43/46/41/123/73/74 + 32/31/1/57/35)
  с фактическими счётчиками трейсов — в README testclient. Попутно починен баг
  entry-point сервера: двойная инициализация tracing паниковала `SetGlobalDefaultError`
  до разбора аргументов (сервер вообще не стартовал); guard-тест
  `crates/server/tests/entrypoint.rs` (сначала падал, теперь зелёный).

## Шаг 10: ванильные трейсы позы и состояний сущностей (14.11.2026)

Шаг 10 подтверждён коммитами `43a8501`, `e6efbc8`, `ea53b2d`, `97f37c9`, `9077ee4`, `36e37b8`, валидными отдельными S7-трейсами и проверками ниже. D (плавание) и playerdata остаются дальнейшими долгами; scope этого шага не расширялся.

| Сценарий | До | После / факты |
|---|---|---|
| S1b — углы при входе наблюдателя (A) | `add_entity` использовал нулевые углы | Vanilla B payload завершает spawn хвостом `00 40 45 45 00` (data/pitch/yaw/head-yaw/trailing); encoder A берёт сохранённые углы. Побайтный тест есть; vanilla S1b `2026-11-14_1831` (`… 00 40 45 45 00`), локальный S1b `2026-11-14_s1b-ours-final` (`… 00 10 f5 f5 00`, yaw/pitch A на момент входа) — оба зелёные. |
| S5 — sneak/sprint (B) | Ввод применялся в состоянии игрока, observer metadata не отправлялась; сценарий проверял только наличие | Ванильные окна подтверждают отдельные flags и pose metadata; encoder и unit-тесты это проверяют. Ours live `2026-11-15_step10-live-ours-s5-observe`: wire checks PASS (`player_input` `20 00 01 41 01 00`, metadata flags `02 → 00 → 08 → 00`, pose `05 → 00`, sprint attributes); B observe подтвердил sneak `CROUCHING/true`, sprint `STANDING/false` (одна попытка на состояние). |
| S3 — swing (C) | `drain_swings` не подключён | B получил 11 пакетов `swing_animation` (id 123, clientbound), все побайтно `8a 08 00 01 06` — VarInt entity id A = `8a 08` (1034), затем `00 01 06` (смысл трёх хвостовых байт **не установлен**). A послал 22 `punch` (46) и 2 `player_action` (41); A получил 0 swing (рассылка идёт только наблюдателям). Сетевая задержка read→write и независимый clock offset проверены: greedy правило `>=150 ms` по A `read_ms` принимает 8 из 22 (seq `27042, 27432, 27918, 28683, 28789, 28954, 29069, 29162`), но B содержит 11 swings. Совпадающие интервалы первых одиночных событий расходятся на `−2.49`, `−0.80`, `+7.53 ms` после offset; B имеет две тройки интервалами `<150 ms` сразу после `block_destruction` (seq 4920–4922 и 5094–5096). Правило не объясняет число/тайминг, C остановлен без encoder. Сведение часов A/B и сырые интервалы — в разделе «S3: сведение часов A/B» ниже. |
| S6 — `player_action` 41 | Причина отсутствия пакета неизвестна | Vanilla S6: miss, survival; клиент не видел блок под прицелом, потому 41 не послал. |
| S5 creative — проверка B gate | Проверен только survival; gate `game_mode == 0` оставлен до отдельной записи | Vanilla stand запущен с `gamemode=creative`, `force-gamemode=true`; лог подтверждает `Default game type: CREATIVE`. A послал held-input flags `20`, `00`, `01`, `41`, `01`, `00`; B для A(entity `0x4f`) прислал sneak `02` (4263) → clear `00` (5926) → sprint `08` (6907) + sprint attribute (6908). Creative transition gate подтверждён для `game_mode == 1`; adventure/spectator не проверялись. Spawn flags/pose добавляются только для подтверждённого S7 survival режима (`game_mode == 0`); creative spawn не выводим из transition-трейса. Run `E:\MineRust-data\autotests\2026-11-14_s5-creative-retry`. |
| S7 — observer joins while A holds state | spawn-state для уже приседающего/бегущего игрока неизвестен | Harness для каждого pose: A входит → stdin `tp Alice 96.5 145 -31.5` → клиент подтверждает позицию → ожидание ≥2 с → runtime-проверка безопасной позиции/блоков → удержание → B входит → анализ только post-login play. Первый прогон `E:\MineRust-data\autotests\2026-11-14_s7-validated` остановлен до ввода/B: клиент остался на исходной unsafe позиции. Второй прогон `E:\MineRust-data\autotests\2026-11-14_s7-rcon-retry` доказал sneak ввод, но завершился на harness false-negative: GUID bytes сравнивались неверно, хотя Alice `add_entity` присутствовал после B login. Конверсия UUID исправлена (`Guid.ToByteArray()` — фактический wire order), spawn matcher ограничен play. Валидный held-sprint trace `E:\MineRust-data\autotests\2026-11-14_s7-sprint-retry`: телепорт ровно в `(96.5,145,-31.5)`, `onGround=true`, feet/head air, support `dark_oak_leaves`, после ожидания ≥2 с; observe подтвердил `up=true`, `sprintKey=true`, `sprinting=true`, `horizontalCollision=false`; A serverbound `player_input` flags `40`→`41`; B после login seq 46 увидел A `add_entity` seq 135 (entity 58, UUID `10 92 05 08 d5 d8 3e ed 93 d2 92 f1 93 af e7 d7`), затем metadata seq 136 (`3a 09 03 41 7a aa ab 10 00 7f ff`) и `update_attributes` seq 137 (полный initial payload сохранён в trace; конкретная sprint-атрибуция для этого пакета не установлена). Эти initial-state пакеты зафиксированы как факты, без предположения, что они вызваны sprint input. Прогон PASS 5/5; `summary.json`/`summary.txt` и обе трассы сохранены. Отдельный held-sneak `E:\MineRust-data\autotests\2026-11-14_s7-sneak-final` также PASS 5/5: та же подтверждённая безопасная позиция; `shift=true`; A flags `20`; B после login увидел A `add_entity` seq 175 (entity 47), metadata seq 176=`2f 00 00 02 06 14 05 09 03 41 7a aa ab 10 00 7f ff`, initial update_attributes seq 177. Metadata содержит флаг `02`; атрибуты сохранены как наблюдаемый initial packet без вывода о причине. Оба S7 трейса отдельные и валидные. `scenario-lib.ps1` теперь ждёт выхода Java после stop, а S7 останавливает оба клиента между позами, устраняя гонку JNA native DLL.

Packet IDs (все — из пользовательского `E:\MineRust-data\reports\packets.json`, `play`): `swing_animation` clientbound 123, `punch` serverbound 46, `player_action` serverbound 41, `update_attributes` clientbound 134, `set_entity_data` clientbound 101.

**Пункт 0b (углы `add_entity` при входе наблюдателя) — закрыт.** Хвост пакета ванильного B: `… 00 40 45 45 00` = `data=0`, `pitch=0x40` (90°), `yaw=0x45`, `head_yaw=0x45`, `trailing=0`. Наш сервер теперь кодирует `pitch/yaw/head_yaw` из сохранённой `PlayerPos` (квантование `pitch_byte`/`byte_angle`), тест `spawn_carries_the_stored_pose_angles` сравнивает payload побайтно с ванильным S1b. Live S1b на нашем сервере: B видит yaw=-15.47°, pitch=22.5° (поза A на момент входа), проверка PASS.

Для B в S5 есть отдельные sneak и sprint переходы; sneak flags/pose пакеты идут отдельно, проверяются оба значения. Формула flags — OR подтверждённых битов `0x02` и `0x08`; комбинация `0x0a` помечена «комбинация не подтверждена трейсом». Не обобщать переходы на другие game modes. S7 held-sneak trace `E:\\MineRust-data\\autotests\\2026-11-14_s7-sneak-final`, B seq 176, содержит initial metadata Alice entity `0x2f`: `2f 00 00 02 06 14 05 09 03 41 7a aa ab 10 00 7f ff`; в одном пакете зафиксированы sneak flags `02`, pose metadata `06 14 05` и исходные snapshot entries. Encoder передаёт текущий sneak state в spawn snapshot только survival (`game_mode == 0`), единственный подтверждённый режим S7; creative spawn и другие режимы не выводятся. Sprint spawn не добавляется, так как единичный sprint trace не доказывает его кодирование. Обычные movement packets остаются в своём потоке. В S7 initial metadata Alice также содержит health `15.67` (float bytes `41 7a aa ab`); найденный `damage_event` адресован entity `0x71`, не Alice `0x2f`, поэтому источник initial health и snapshot attribute values остаётся не установлен. Порядок подтверждён только для sprint и только относительно движения той же сущности: `entity_position_sync` (4028) → metadata (4029) → attribute (4030), и `move_entity_pos` (6957) → metadata (6958) → attribute (6959). У sneak соседние пакеты принадлежат другим сущностям (`set_entity_motion` 2218 и `move_entity_pos_rot` 2219 — entity `0x40`; `move_entity_pos` 3327 — entity 129), поэтому **порядок sneak относительно собственного движения трейсом не установлен**; encoder применяет подтверждённый sprint-порядок (движение перед metadata) ко всем переходам. Live run на нашем сервере подтвердил тот же порядок: sprint-off move 1229 → metadata 1230 → attribute 1231. См. ванильный S5 `E:\\MineRust-data\\autotests\\stage10-s5-vanilla-sprint-forward-2026-11-14\\mcproxy\\proxy-conn-2.jsonl`.

Источники A: `E:\\MineRust-data\\autotests\\2026-11-14_1831\\mcproxy\\proxy-conn-2.jsonl` (vanilla) и `E:\\MineRust-data\\autotests\\2026-11-14_s1b-ours-final\\mcproxy\\proxy-conn-2.jsonl` (ours). Источники C: S3 `E:\\MineRust-data\\autotests\\2026-11-14_1644\\mcproxy\\proxy-conn-6.jsonl` (A) и `proxy-conn-7.jsonl` (B). Отсчёты `read_ms` внутри одного connection независимы от другого; способ их сведения описан ниже и применим только к сохранённым прогонам.

Gate шага 10: `cargo fmt --all`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace` — зелёные; `run-scenario.ps1` проходит PowerShell parser, `git diff --check` чист. Ours live S5 `2026-11-15_step10-live-ours-s5-observe` прошёл wire и remote-pose checks; S7 `2026-11-15_step10-live-ours-s7-retry2` подтвердил обе удалённые позы; S1b `2026-11-14_s1b-ours-final` PASS.

### Сведение часов A/B (метод и границы)

`read_ms` в `proxy-conn-N.jsonl` считается `Instant::now()` от старта **своего**
соединения (`crates/parity/src/proxy.rs`, `serve` → `started`), поэтому два
connection несравнимы напрямую. Для сохранённых прогонов доступна внешняя
привязка: строка `INFO parity::proxy: recording …proxy-conn-N.jsonl` пишется
в `mcproxy-err.log` непосредственно перед `Instant::now()`, так что её
wall-clock метка даёт начало отсчёта connection с точностью до задержки
записи лога (единицы мс). Сведение: `t_abs(p) = anchor[N] + read_ms(p)`;
межсоединительная дельта — разность `t_abs` двух пакетов. Метод валиден
только для этих прогонов, требует `mcproxy-err.log` рядом с трейсами и не
является частью протокольного сравнения; в отчётах он всегда помечается как
внешняя привязка, а не как «общие часы».

Привязки сохранённых прогонов (мс, unix-время): S3 `2026-11-14_1644`
conn6 = `…13:48:05.893`, conn7 = `…13:48:38.295` → сдвиг **32401.66**;
S5 `2026-11-15_step10-live-ours-s5-observe` conn1/conn2 → **36921.50**;
S7 `2026-11-15_step10-live-ours-s7-retry2` conn1 `09:56:50.754`, conn2
`09:57:28.977`, conn3 `09:58:06.393`, conn4 `09:58:39.074`.

### S3: сырые последовательности A и B

Ниже — всё, что есть в трейсах: A (`punch` serverbound 46,
`player_action` serverbound 41) и B (`swing_animation` clientbound 123,
`block_destruction` clientbound 5). Время — мс от первого `punch` A
(`read_ms` A первый = 38694.9654); B пересчитан по привязке выше
(сдвиг 32401.66). Помечено: `P` — `punch`, `A` — `player_action`,
`S` — `swing_animation`, `D` — `block_destruction`; перед каждым — `seq`.

A (22 `punch`, 2 `player_action`):

```text
27042:P:0.000 27432:P:653.309 27918:P:1301.664 28682:A:2398.632
28683:P:2398.840 28684:P:2399.985 28707:P:2429.955 28741:P:2505.792
28743:P:2506.046 28789:P:2656.473 28791:P:2657.170 28869:P:2665.197
28871:P:2699.553 28889:P:2750.719 28915:P:2804.784 28954:P:2851.424
28980:P:2902.285 29011:P:2947.890 29047:P:3001.111 29069:P:3045.768
29097:P:3099.188 29137:P:3153.391 29161:A:3201.955 29162:P:3202.312
```

B (11 `swing_animation`, 11 `block_destruction`):

```text
3130:S:48.055 3594:S:698.871 4142:S:1346.426 4919:D:2450.775
4920:S:2451.131 4921:S:2451.227 4922:S:2451.323 4952:D:2470.944
5027:D:2686.761 5092:D:2709.355 5094:S:2709.626 5095:S:2709.732
5096:S:2709.836 5142:D:2765.887 5185:D:2819.623 5213:S:2897.254
5241:D:2918.398 5307:D:3015.345 5386:D:3125.134 5393:S:3146.834
5444:D:3217.387 5452:D:3246.890
```

Payload `block_destruction` (все 11): `8a 08` (entity A = 1034) + packed
block pos `00 00 17 7f ff fe a0 7b` (x=93, y=123, z=−22) + stage `00..09`,
затем `ff` (сброс). `swing_animation` во всех 11 пакетах — `8a 08 00 01 06`.

Что видно из этих чисел (только факты, причина не выводится): одиночные
`S` идут через ~48/45/45 мс после ближайшего `P`; две тройки `S` (4920–4922,
5094–5096) идут с шагом 0.10 мс внутри тройки сразу после `D` (4919, 5092);
между `D` идут серии 21–106 мс. Связь «`P` → `S`» и правило агрегации
(11 против 22) из данных не следуют; C по-прежнему не кодируется.

### S7: аудит здоровья A до входа B

Трейс `E:\\MineRust-data\\autotests\\2026-11-14_s7-sneak-final\\mcproxy\\proxy-conn-1.jsonl`
(ваниль). Окно — всё до `player_info_update` с UUID Bob (seq 23607,
`read_ms` 38617.702):

- `entity_event` seq 51 (`read_ms` 1769.665) — `00 00 00 2f 18` (entity 0x2f,
  status 0x18); других `entity_event` для `0x2f` до входа нет.
- `player_position` (clientbound) ровно два: seq 55 (`read_ms` 1770.413,
  flags 1, `y = 128.0`) и seq 3808 (`read_ms` 6997.654, flags 2,
  `y = 145.0`); падения Y между ними нет.
- `set_health` ровно один: seq 160 (`read_ms` 1863.106),
  `41 7a aa ab 11 00 00 00 00` = health 15.666667, food 17, saturation 0.
- `damage_event` до входа B — **0**; единственный `damage_event` прогона
  seq 27142 (`71 06 00 00 00`) приходит позже и адресован entity `0x71`,
  не Alice `0x2f`.

Отсюда: видимое падение здоровья фиксируется только пакетом `set_health`;
пакета, который бы его вызвал, в трейсе нет. Источник значения 15.67
**не установлен**, спавн-код его не воспроизводит (в live-трейсах `ours`
`set_health` и `damage_event` = 0 во всех connection).

**Уточнение (шаг 12, 15.11.2026, факт владельца).** Значение совпадает с
сохранённым player state ванильного мира: `Health` в
`players\data\10920508-...-92f193afe7d7.dat` рабочего ванильного мира =
15.666666984558105, `foodLevel` = 17 (см. П.1a шага 11). Вход A в S7 произошёл
на `y = 128` — ровно `Pos.y` из `...dat_old` (прежняя, ещё не
телепортированная позиция). Значит ванильные S7-трейсы шли **с сохранённым
состоянием игрока** (ваниль пишет playerdata); `set_health` в трейсе — это
синхронизация уже сохранённого здоровья, а не расхождение нашего сервера.
Наш сервер playerdata не пишет и `set_health` не шлёт; вопрос источника 15.67
закрыт как «сохранённое состояние ванильного мира».

### Live-задержки S5/S7 (внутри и между connection)

S5 `2026-11-15_step10-live-ours-s5-observe` — от serverbound `player_input`
A до clientbound пакета B (привязка conn1/conn2, сдвиг 36921.50):

| Переход | A seq (flags) | B flags | Δ, мс | B pose | Δ, мс |
|---|---|---|---|---|---|
| sneak-on | 2127 (`20`) | 640 | 60.08 | 641 | 60.19 |
| sneak-off | 2163 (`00`) | 814 | 50.41 | 815 | 50.49 |
| sprint-on | 2184 (`41`) | 904 | 16.18 | 905 | 16.34 |
| sprint-off | 2296 (`01`) | 1211 | 33.81 | 1212 | 33.87 |

S7 `2026-11-15_step10-live-ours-s7-retry2` — тот же вход B (клиент принят,
`add_entity` A уже в потоке):

| Сценарий | accept → `add_entity` | +metadata seq 61 | +metadata seq 63 | +`update_attributes` seq 64 |
|---|---|---|---|---|
| held-sneak | 869.704 | 0.060 | 0.195 | 0.279 |
| held-sprint | 882.333 | 0.095 | 0.273 | 0.352 |

Числа получены одной и той же внешней привязкой по `mcproxy-err.log`; это
не «задержка сервера», а верхняя граница наблюдения на проводе.

### Сценарии до/после

| Сценарий | До | После изменений |
|---|---|---|
| S1b | углы AddEntity не передавались | A закодирован; побайтный unit test и сохранённые vanilla/ours traces зелёные |
| S5 | факт наличия общих metadata пакетов | точные sneak flags `02 → 00`, отдельные pose `05 → 00`, sprint flags и attribute payloads проверяются; ours live `2026-11-15_step10-live-ours-s5-observe` также проверил wire последовательность и B remote pose через observe |
| S3 | vanilla swings были только диагностикой, ours не передавал их | факты дополнены; C не кодируется из-за неоднозначности частоты/связи |
| S6 | диагностика причины отсутствующего 41 | confirmed: crosshair miss в survival |

## Шаг 11: read-only диагностика миров и защита runner'а (15.11.2026)

Шаг открыт после того, как прогон `2026-11-15_1908` дал три красных S5-проверки,
а в ванильном мире была найдена сохранённая playerdata Alice. Мир не удалялся и
не изменялся: всё ниже — чтение. Диагностика предшествует одной live-попытке S5;
выводы без подтверждённого прогона помечены как гипотезы.

### П.1a — NBT Alice в `E:\MineRust-data\vanilla\world` (только чтение)

UUID `10920508-...-92f193afe7d7` — та же Alice, что в сценариях. Источник —
рабочий ванильный мир (`vanilla\world`), **не шаблон**; файлы не изменялись.

| Файл | mtime | Health | foodLevel | Pos | Rotation |
|---|---|---|---|---|---|
| `players\data\10920508-...dat` | 2026-11-14T23:36:11 | 15.666666984558105 | 17 | `[96.5, 145.0, -31.5]` | `[0.0, 0.0]` |
| `players\data\10920508-...dat_old` | 2026-11-14T23:02:38 | 15.666666984558105 | 17 | `[96.5, 128.0, -10.300000011920929]` | `[0.0, 0.0]` |

Рядом лежит `faa5dca3-c3d4-354b-ae1b-dde9e5a14b3b.dat` (Bob). Значения
совпадают с тем, что ванильный сервер присылал live (`set_health` 15.666667,
food 17 — раздел «S7: аудит здоровья A»): ванильный сервер **сохраняет** player
state, и `Health`/`foodLevel`/`Pos` переживают прогоны. Значения `Pos` из
`.dat_old` (y=128, z=−10.3) — прежняя, ещё не телепортированная позиция, отсюда
разница с `.dat` (y=145, z=−31.5) и с вводом сценария.

### П.1b — какой мир использовал каждый прогон

Различитель в логе: ваниль печатает `Preparing level "world"`, наш сервер —
`world configured ... dir=`. `E:\MineRust-data\world` как каталог отсутствует,
поэтому все прогоны делятся на две группы.

| Прогон | Цель | Мир | «Возможен перенос состояния» |
|---|---|---|---|
| `2026-11-14_1831` (S1b) | vanilla | `E:\MineRust-data\vanilla\world` | да |
| `2026-11-14_1843` (S5) | vanilla | `E:\MineRust-data\vanilla\world` | да |
| `2026-11-14_s5-creative` / `-retry` | vanilla | `E:\MineRust-data\vanilla\world` | да |
| `2026-11-14_s7-sneak-final` | vanilla | `E:\MineRust-data\vanilla\world` | да |
| `2026-11-14_s7-sprint-retry` | vanilla | `E:\MineRust-data\vanilla\world` | да |
| `2026-11-14_1900` (S6) | ours | `E:\MineRust-data\world-template-v3` | нет (см. П.1c) |
| `2026-11-14_s1b-ours-final` | ours | `E:\MineRust-data\world-template-v3` | нет |
| `2026-11-15_1903` | ours | `E:\MineRust-data\world-template-v3` | нет |
| `2026-11-15_1908` | ours | `E:\MineRust-data\world-template-v3` | нет |
| `2026-11-15_step10-live-ours-s5-observe` | ours | `E:\MineRust-data\world-template-v3` | нет |

Ранние S1b/S5/S6 (14.11) уже были разделены по цели; до Шага 11 runner не
пересоздавал мир и не аудировал его, поэтому у ванильных прогонов состояние
могло переноситься между запусками (см. П.3).

### П.1c — mtime `level.dat` и `session.lock`

- `E:\MineRust-data\vanilla\world\level.dat` — **2026-11-14T23:36:24.258+03:00**,
  `session.lock` — **2026-11-14T23:34:52.043+03:00**: почти на 19.5 ч раньше
  15.11 19:08. В прогоне `2026-11-15_1908` ванильный мир не участвовал.
- `E:\MineRust-data\world-template-v3\level.dat` — 2026-11-09T22:48:06,
  `session.lock` — 2026-11-09T22:45:34: не менялись ни одним ours-прогоном.
  Наш сервер не содержит кода сохранения мира/playerdata (в `crates/net`
  нет `playerdata`/`save-all`/`write_player`), поэтому ours-прогоны работают
  с шаблоном только на чтение и состояние между ними не переносят.
- playerdata в шаблоне — один файл, `ff1070c9-7b1a-3e30-a01e-89fe7a426781.dat`,
  чужой UUID (не Alice `10920508-...`): собственной Alice в шаблоне нет.

### П.1d — прогон `2026-11-15_1908`: цель и отсутствие sprint-off

Цель — **ours**, мир `E:\MineRust-data\world-template-v3`; ванильный мир в этом
прогоне не участвовал. Факты из `proxy-conn-2.jsonl`:

- metadata-флаги Alice `02 → 00 → 08` есть (seq 542/744/847), `08` — sprint-on.
- `update_attributes` с `minecraft:sprinting` присутствует: seq 849,
  `01 01 1a 3f b9 99 99 a0 00 00 00 01 13 ... 73 70 72 69 6e 74 69 6e 67 ...`
  (проверка сообщала «missing» из-за matcher'а, а не из-за отсутствия пакета;
  см. ниже).
- sprint-off (flags `00` и sprint-off attribute) в B-трейсе отсутствуют.
- B-трейс обрывается на seq 946, `read_ms` 14.441 s; A-трейс дошёл до ~83.48 s.
  Обе трассы закрыты одновременно в 19:12:56 при `disconnect`. Последние
  события B — `level_chunk_with_light`/`chunk_batch_*` (ещё дренировалась
  входная выдача чанков), а не `remove_entities`.

Отсюда **гипотеза** (не факт): sprint-off не «не отправлен сервером», а не
успел попасть в B-трейс, потому что сценарий отключал клиентов, пока B ещё
разгружал входные чанки; в логе сервера переход сгенерирован (тик 2771,
`n_entities=2`, ~+0.49 s после ввода A). Гипотеза подтверждается/опровергается
live-попыткой П.4. В 1908 `player_input` A совпадает с зелёным прогоном
(`20 00 01 41 01 00`), поэтому менять сценарий оснований нет.

### П.2 — защита runner'а от переноса состояния мира (коммит `5eedef7`)

Найдено и закрыто в `tools/testclient/run-scenario.ps1`:

- перед стартом мир **пересоздаётся из шаблона v3** (`$templateWorldDir`);
  старая папка не удаляется, а переименовывается в `world.stale-<yyyyMMdd-HHmmss-fff>`;
- до запуска пишется `world-audit.json`: SHA-256 файлов мира, список
  `players\data\*.dat` шаблона, имена неожиданных файлов; `runStartedAt` и
  `worldAudit` попадают в `summary.json`;
- если в мире есть `players\data\*.dat`, которых нет в шаблоне, runner падает
  с понятной ошибкой **до** входа клиентов;
- `world_dir` в `server-ours.toml` подменяется на активный мир прогона;
- тест `tools/testclient/tests/world-protection.Tests.ps1` (PowerShell parse +
  ротация/аудит/guard, включая ours-ветку с `world.stale-*`) — зелёный.

### П.3 — что означает «возможен перенос состояния»

Пометка в таблице П.1b применима только к ванильным прогонам: ванильный сервер
пишет player state в `vanilla\world`, поэтому между его запусками могли
переноситься:

- **Health** (в NBT зафиксировано 15.666666984558105);
- **Rotation** (`[0.0, 0.0]` в сохранении Alice);
- **Pos** (`[96.5, 145.0, -31.5]` в `.dat`, `[96.5, 128.0, -10.3]` в `.dat_old`).

Следствие: спавн Alice в ванильных прогонах мог начинаться не с нуля, а с
сохранённых значений (позиция/здоровье), в том числе при первом наблюдении B.
Ours-прогоны этого не наследуют (П.1c), но до `5eedef7` runner не гарантировал
это явно — теперь гарантирует. Ни один сохранённый трейс **не объявляется
недействительным** только из-за пометки: расхождения необходимо подтверждать
данными конкретного прогона; для `2026-11-15_1908` данных о переносе нет
(мир-шаблон без playerdata Alice).

### П.4 — live-попытка S5 на чистом мире (`2026-11-15_s5-live-clean`, `ours`)

Одна попытка по требованию; цель та же, что в 1908. Прогон:
`E:\MineRust-data\autotests\2026-11-15_s5-live-clean`, `target=ours`,
`runStartedAt=2026-11-15T21:07:20.7006479+03:00`, мир — пересозданный
`E:\MineRust-data\world` из `world-template-v3`.

Чистота мира подтверждена аудитом: 55 файлов с SHA-256, `playerData` = шаблонный
`['ff1070c9-7b1a-3e30-a01e-89fe7a426681.dat']`, `templatePlayerData` — тот же
список, `unexpectedPlayerData` = `[]`. Переноса состояния нет.

Итог: **11 из 12 проверок PASS**, один FAIL.

| Проверка | Итог | Факт |
|---|---|---|
| A `player_input` flags `20 00 01 41 01 00` | PASS | совпадает с зелёным прогоном шага 10 |
| metadata flags `02 → 00 → 08 → 00` (entity 1) | PASS | seq 1528/1567/1589/1677, hex `01 00 00 02 ff`, `…00 ff`, `…08 ff`, `…00 ff` |
| sneak poses `05` → `00` | PASS | seq 1529 `01 06 14 05 ff`, seq 1568 `01 06 14 00 ff` |
| B observe sneak | PASS | `CROUCHING / isCrouching=true / isSprinting=false` |
| B observe sprint | PASS | `STANDING / false / true` |
| observer chunk drain settled | PASS | `loadedChunks=473; reads=58; settled=true` |
| sprint-on attribute | PASS | seq 1590 `01 01 1a 3f b9 99 99 a0 00 00 00 01 13 … 73 70 72 69 6e 74 69 6e 67 …` |
| sprint-off attribute | PASS | seq 1678 `01 01 1a 3f b9 99 99 a0 00 00 00 00` |
| B trace window ±30 вокруг release | PASS | окно 1647..1707: metadata `01 00 00 00 ff` @1677 и attribute @1678 |
| B observe sprint-off at +0 ms | **FAIL** | `actualAfterReleaseMs=8; found=True; isSprinting=True` (на +158.6 мс уже `False`) |

**Класс причины: баг проверки.** Сервер sprint-off отправил: и metadata
(`01 00 00 00 ff`, seq 1677), и attribute (seq 1678) присутствуют в B-трейсе.
Проверка же использовала сэмпл `+0 мс` как дедлайн, а сервер применяет
отпускание на следующем тике (≤50 мс при 20 TPS) плюс доставка — в момент
`+8 мс` флаг объективно ещё `true`. Тем самым подтверждена гипотеза П.1d:
в `2026-11-15_1908` sprint-off не попал в трейс из-за обрыва трасс при
`disconnect`, а не потому что сервер его не отправил. Сценарий менять не нужно
(`player_input` A идентичен зелёному прогону).

Фикс (сначала падающий тест):
`Get-SprintOffObservation` сворачивает таймлайн в один факт — проверка проходит,
если **любой** поздний сэмпл видит `isSprinting=false`; сэмпл `+0 мс` остаётся в
факте как элемент таймлайна. Таймлайн и сами наблюдения пишутся в `summary.json`
(`sprintOffObservations`). Тест `tools/testclient/tests/s5-attribute.Tests.ps1`
содержит три случая: поздний sprint-off проходит; `+0 мс` в одиночку — нет;
никогда не найденный наблюдатель — нет. Переигровка новой проверки на
**записанных** наблюдениях этого же прогона (без второго live-запуска):
`observed=True`, таймлайн `+0/8ms=True/True; +100/158,6ms=True/False`.

Открытых долгов по S5 после шага не остаётся; `S6`/`S3` (C) — как и были,
вне scope этого шага.

## Шаг 12: возврат к серверу — реестр механик и планы (15.11.2026)

Плановый шаг: код только для ручного `tools\testclient\client.bat` и
документации. Сервер и тест (в) не менялись.

**Ручная проверка владельца (наш сервер, два клиента, survival) — подтверждено:**

- видны: угол при входе, присед, присед при входе к уже приседающему,
  спринт (с частицами), взмах руки в воздух и при ломании блока —
  **дефекты A, B, C закрыты**;
- не видны: **плавание и ползание (D)**;
- **ломание блока не реализовано**: у копающего клиента блок исчезает и через
  него можно ходить (гипотеза владельца, не проверена: клиентский прогноз,
  сервер не подтверждает). Наблюдатель блок не теряет и трещин не видит.
  После перезахода копающего блоки возвращаются, игрок появляется на спавне
  (playerdata не реализован — известный долг);
- **S6 красный по причине стенда** (причина не исследована) — записано как
  есть, без вывода о сервере;
- решения владельца: делаем все механики постепенно, ничего не упускать;
  режимы — survival основной, creative тоже.

Реестр механик — `docs/MECHANICS.md`; план ломания — `docs/BLOCK-BREAKING.md`;
план плавания — `docs/SWIMMING.md`.

## Шаг 13a: общее хранилище блоков `ChunkStore` (15.11.2026)

Код: `net::store::ChunkStore` — единственный владелец текущего снимка
wire-мира; схема владения/конкурентности — `docs/DECISIONS.md`, «Шаг 13a».

- `ReadyChunks` теперь держит `Vec<Arc<LevelChunkWithLight>>`; новый
  `ReadyChunks::with_chunk` подменяет один payload, остальные — те же `Arc`.
- `ChunkStore { current: RwLock<Arc<ReadyChunks>>, blocks: Arc<BlockStates> }`:
  `snapshot()` — клон указателя под коротким чтением; `block`/`set_block`
  читают/пишут блок по координате. `set_block` пересобирает **только**
  затронутый чанк (распаковка/перепаковка одной секции, палитра только
  растёт, заголовки `non_air`/`fluid` правятся) и публикует новый снимок.
- `ChunkSource::load_batch` отдаёт `Arc<ChunkStore>`; `TemplateWorld.ready`
  — `OnceLock<Arc<ChunkStore>>`. `PendingBatch` берёт `snapshot()` на каждый
  подпакет, поэтому вошедший позже видит изменённый блок.
- `data::BlockStates`: добавлены `properties` и `is_air`/`is_fluid` (правило
  дублирует `world::wire_section::counts_as_fluid`, SOURCE — `parity/a2_findings`).
  `WireTables.blocks` стал `Arc<BlockStates>` (общий с хранилищем).

**Замеры (release, v3, `cargo test -p parity --release --test store_rebuild
-- --ignored --nocapture`):**

| Величина | Значение |
|---|---|
| warm-up (сборка + чтение 4481 чанка) | 8424 мс |
| вход второго клиента (сторона store: кэш + snapshot), p50 | 15.1 мкс |
| пересборка одного чанка, первый write (`Single` → packed) | 182.5 мкс |
| пересборка одного чанка, установившееся p50 | 134.9 мкс |

Второй клиент меряется на стороне store — то, что изменил 13a; сетевой вход
не включается.

**Тесты.** `net::store`: чтение/запись, адресная инвалидация (соседний чанк —
тот же `Arc`), заголовки секции, палитра, ошибки; `net::chunks::with_chunk`;
`net::connection::play::tests::a_sender_after_a_block_change_reads_the_new_block`
— sender после записи отдаёт изменённые байты чанка.

**Не установлено / границы.** Block entity изменённого блока не правится;
палитра секции не чистится; свет не пересчитывается; на диск не пишется
(только память). Игровой поток пока не получает `Arc<ChunkStore>` — это
подключит Шаг 13b.

## Шаг 13b: серверное ломание блока в survival (15.11.2026)

Код: `net::game::block_action` + декодеры в `crates/protocol`. Схема —
`docs/DECISIONS.md`, «Шаг 13b»; байты и гипотезы — `docs/BLOCK-BREAKING.md`
§3.5.

- `protocol`: `PlayerAction` (41, serverbound), `BlockUpdate` (8),
  `BlockChangedAck` (4); экспорт и `used_packet_keys`. Побайтные тесты против
  S3: `00 0000177ffffea07b 01 01`, `03 0000177ffffea07b 01 02`,
  `0000177ffffea07b 00`, ack `01`/`02`.
- `net`: `ToGame::BlockAction(BlockAction)`; `ServerMessage::BlockUpdate`
  / `BlockChangedAck`; соединение пишет их, резолвя id через `ids.id(...)`.
- `game`: `GameConfig.store: Option<Arc<ChunkStore>>` (конфиг больше не
  `Copy`, у `ChunkStore` ручной `Debug`); `warm_world` в `server` возвращает
  store и кладёт его в конфиг.
- Поведение: start/abort — только ack; finish — проверка «чанк загружен и
  блок не воздух», запись `AIR_STATE=0`, `block_update` всем (включая
  копающего), затем ack копающему. Отказ: воздух → `block_update` реального
  состояния копающему + ack; нет чанка → только ack. Всё это **гипотеза**
  (трейса отказа нет), в коде помечено.

**Тесты.** `protocol`: 2 побайтных теста S3. `net`: 7 тестов
`block_action_tests` (успех и порядок, наблюдатель без ack, поздний вход
читает новый снимок, воздух, отсутствующий чанк, start/abort, без store,
путь через `ToGame`). Полный gate (`fmt`, `clippy -D warnings`, `test
--workspace`) — зелёный.

**Не сделано (осознанно).** Трещины, звук/частицы, предмет,
`block_destruction`/`level_event`/`add_entity`; пересчёт света; таймер
копания, твёрдость, проверка дальности; правило «кому видно»
(`block_update` — всем); запись на диск. Проверка света после `block_update`
по ванильному трейсу — **не установлено**.

## Шаг 13c: ванильные трейсы S8 (вода) и S9 (creative-ломание) (16.11.2026)

Кода нет — только стенд и доки. Сценарии `tools/testclient/scenarios/
s8-water.json` и `s9-creative-break.json`, функции `Invoke-S8`/`Invoke-S9` в
`run-scenario.ps1` (на `-Target ours` — честный пробел «нет RCON»).
Факты: `docs/SWIMMING.md` §0.2, `docs/BLOCK-BREAKING.md` §1.5, `docs/
MECHANICS.md` (пп. 4, 5, 6, 12).

- **S8** (`2026-11-16_0918`, 6/6 PASS): спринт по земле → RCON `/tp` в воду
  (106.5, 41, −30.5) → спринт-заплыв → обратный `/tp`. Факты: pose SWIMMING
  (varint 3) уже в покое; flags `0x10`/`0x18`/`0x00`; air supply (index 1)
  299→201 по 1/тик под водой, регенерация после всплытия; `oxygen_bonus`
  (key 28) = 0.0 при первом погружении; `water_movement_efficiency` (37) не
  отправляется; звук входа/выхода из воды отсутствует (единственный sound —
  `block.gravel.break` при приземлении после tp).
- **S9** (`2026-11-16_0932`, чек сценария переписан; в 0918 первая попытка):
  creative = `player_action 0x00` (start) на каждый блок → сразу
  `block_update` state 0 + `block_changed_ack`; `0x03`/`0x02` в creative не
  шлются. Наблюдателю: `level_event` 2001 (data int32 = block state id
  ломаемого блока: `450` = dark_oak_leaves, сходится с S3 `9` = grass_block) +
  `swing_animation`; `block_destruction`/`sound`/дропы нет. Ваниль сломала два
  листа на y=143 рядом со спавном, поэтому A не падал — чек «y должен упасть»
  заменён на «crosshair сменил цель».
- **Стенд-факт:** одиночный тап attack не отправляет ни одного пакета даже на
  ванили при валидной цели (2 прогона); пакеты даёт только удержание.
  Флейк-чек «B chunk drain settled» переведён из проверок в facts (ронял
  прогоны 0928/0932 без протокольной причины).

## Шаг 14b: автоочистка world.stale (16.11.2026)

Коммит `1fca1d9`. Превенторный запуск (`Prune-StaleWorlds` в `run-scenario.ps1`
перед `Prepare-RunWorld`): после копирования рабочего мира удаляются устаревшие
каталоги `world.stale-<yyyyMMdd-HHmmss-fff>` в этом же каталоге, **остаются
два последних**. Защита: имя обязано совпасть с лист-шаблоном, а `resolve()`
после разрешения — оставаться внутри родительского каталога (никогда не удалять
не-`world.stale` и ничего вне `world*`). Тест `world-protection`: три
`Prepare-RunWorld` подряд → два stale каталога оставлены.

## Шаг 14c: creative-ломание + `level_event` 2001 (16.11.2026)

Код: `crates/protocol` (`LevelEvent` id 47, `EVENT_BLOCK_BREAK = 2001`,
`TRACED_TRAILING = 0x00`), `crates/net/src/game/block_action.rs`
(`broadcast_break`, creative start → finish), `ServerMessage::LevelEvent`,
`GameConfig::default_game_mode` (server.toml `game_mode`, 0..=3, default 0),
runner `-OursGameMode`. Детали и байты: `docs/BLOCK-BREAKING.md` §1.6.

- Creative = mode 1: `player_action 0x00` (start) ломает блок мгновенно —
  `level_event` 2001 наблюдателям → `block_update` state 0 всем →
  `block_changed_ack` копающему. 2001 отправляется и в survival (S3).
  `2019`/`2020`/`block_destruction` не шлются.
- Побайтные тесты S3 (`000007d1 0000177ffffea07b 00000009 00`) и S9
  (`000007d1 0000183ffffe208f 000001c2 00`); 10 тестов `block_action`.
- **Живой прогон S9 на нашем сервере** (`2026-11-16_1233`, `-OursGameMode 1`):
  5/5 PASS — creative виден B, прицел на блоке (96,143,−30), удержание удалило
  блок (crosshair → MISS), B видит A, **`level_event` 2001 приходит раньше
  air-`block_update` (ordered=True)**. Байты B совпали с ванильным S9.
  Прогоны 1107/1145 — все модовые чеки PASS (B-trace 57 МБ читался python'ом;
  чек порядка в runner поправлен на попарное сравнение).

## Шаг 14d: плавание — water state у ног → поза/flags (16.11.2026)

Код: `crates/data` (`BlockStates::is_water` — только `minecraft:water`, не
waterlogged), `crates/net/src/store.rs` (`ChunkStore::is_water_at`),
`crates/net/src/game/scheduler.rs` (`update_swimming` — проба `floor(x),
floor(y), floor(z)` по общему снимку перед visibility pass),
`crates/net/src/game/region.rs` (`Player::swimming`),
`crates/net/src/game/entities.rs` (`EntitySpawn::swimming`, pose/flags при
спавне и переходе), `crates/net/src/entity_packets.rs` (побайтные тесты),
`crates/server` (dev-параметр `spawn_override`), runner `-OursSpawnOverride`.

- Правило (`docs/SWIMMING.md` §2): swimming = блок у ног — вода; pose `3` в
  воде / `0` вне; flags = sneak `0x02` | sprint `0x08` | swimming `0x10`. Первый
  покойный сэмпл S8 у ванили был `0x00`, у нас выходит `0x10` — расхождение
  задокументировано явно, без одноразового исключения.
- Проба у ног — `floor(y)` (блок, где начинается AABB игрока). Первая попытка
  использовала `floor(y − 0.1)` и на live-прогоне ошиблась: у пловца, стоящего
  на дне пруда, берётся блок *под* ногами (твёрдый), swimming ложно `false`.
- Побайтные юнит-тесты:
  `entity_packets::tests::swimming_transitions_match_the_s8_metadata_bytes`
  (flags `0x10`/`0x18` + pose `03`, сброс `00` + pose `00`) и
  `entities_tests::traced_swimming_pose_is_three_and_exit_is_standing`.
- **Живой прогон S8 на нашем сервере** (`2026-11-16_1420`,
  `-OursSpawnOverride 112,64,24`, 2/2 PASS): A входит из воздуха в воду
  (feet/head = `minecraft:water`, `y=48`), B видит A `pose=SWIMMING`. Байты B:
  `01 00 00 10 06 14 03 ...` (flags SWIMMING + pose 3) и отдельный
  `... 09 03 41 a0 00 00 10 00 7f ff`. Точка входа выбрана по NBT копии мира:
  океан `y=59..62`, воздух с `y=63`; ориентир плана `(106,41,−31)` — закрытый
  пруд (вода `y=37..41`, камень `y=42`, глубина 5), над ним в воздух не встать.
- `spawn_override` меняет **только** стартовую позицию нового игрока (join
  teleport и центр окна); `Set Default Spawn Position` остаётся спавном
  сохранения, шаблон v3 и файлы мира не трогаются. Точка проверяется по
  готовому `ChunkStore`; при неготовом чанке — warning и обычный спавн.
- **Не реализовано (явно):** таймер воздуха и урон, подводность глаз,
  `oxygen_bonus`/`water_movement_efficiency`, всплытие прыжком, звуки/частицы
  всплесков, ползание.

## Шаг 14e: правило плавания из ванильного S10-прогона (16.11.2026)

Black-box вывод правила позы/flags по ванильным трейсам; чужой код не читался.
Стенд и разбор: `tools/testclient/scenarios/s10-swimcrawl.json`,
`tools/trace-analyze/s10-swim-table.py`, `tools/trace-analyze/s10-rule-search.py`.
Трейс: `E:\MineRust-data\autotests\s10-vanilla-r3` (16 геометрий G1..G7, 47
сэмплов на геометрию, 0 failed). Часы A/B сведены по строкам `recording ...
proxy-conn-N.jsonl` в `mcproxy-err.log` (сдвиг B→A = +35944.8 мс).

- **Таблица решений (по r3).** По воде у ног, битам `player_input` (sprint,
  sneak) и метаданным, которые получил B: `swim = feet_in_water ∧ sprint`;
  `flags = sneak?0x02 | sprint?0x08 | swim?0x10`; `pose = swim?3 : sneak?5 : 0`.
  Бит `SWIMMING` «липкий» на одно событие (сетевой лаг на обоих фронтах).
- **Вода у ног — по высоте жидкости, не по имени блока.**
  `BlockStates::water_surface`: источник `minecraft:water` = `(8 − level)/9`,
  waterlogged-блок и 5 растений (`seagrass`, `tall_seagrass`, `kelp`,
  `kelp_plant`, `bubble_column`) = `8/9`; lava/воздух — не вода. Поэтому игрок,
  стоящий **на** waterlogged-блоке, сухой, а стоящий **в** нём — мокрый (прежнее
  правило 14d «блок у ног — `minecraft:water`» этого не различало).
- **Oracle.** `crates/net/src/game/swim_tests.rs` прогоняет 68 записанных
  flags-переходов через `is_swimming` + `state_flags`; фикстура
  `crates/net/src/game/s10_fixture.rs` сгенерирована `s10-rule-search.py`.
  Результат: **точно 50/68 (73.5 %), в пределах ±1 события 66/68 (97.1 %)**.
  Два расхождения — транзиенты sprint-флага на телепорте (`G5-bubble-column`,
  `G5-seagrass`): наш `0x08` против ванильного `0x00`. Три формы правила
  (simple / eyes / hysteresis) дали одинаковые 66/68 — данные не различают,
  оставлена простейшая.
- Код: `crates/net/src/game/entities.rs` (`is_swimming`, `pose_of`),
  `crates/net/src/game/scheduler.rs` (`update_swimming` — правило 14e вместо
  временного гейта), `crates/net/src/store.rs` (`water_surface_at`,
  `feet_in_water`), `crates/data/src/blocks.rs` (`WATER_PLANTS`,
  `is_waterlogged`, `water_surface`).
- **Не установлено:** подводность глаз (старт только с глазами под водой данными
  r3 не различается — не реализована), ползание (туннели G7 дали только явный
  sneak `0x02`/pose 5; автоматической позы ползания нет — нужен отдельный
  ручной сценарий). **Ползание закрыто на шаге 15:** правило выведено
  S11-прогоном (см. §5 `docs/SWIMMING.md` и раздел «Шаг 15» ниже); подводность
  глаз остаётся долгом (шаг 15 п.6).
- **Live S8 на нашем сервере** (`E:\MineRust-data\autotests\s8-ours-14e`,
  `-OursSpawnOverride 106,41,-31`, 3/3 PASS): A входит в пруд (feet/head =
  `minecraft:water`, `y=37`, стоит на дне), **в покое B видит `pose=STANDING`,
  `isSwimming=false`**, затем A жмёт forward+sprint через KeyMapping и **B видит
  `pose=SWIMMING`, `isSwimming=true`** (`pos` сместилась на z на 0.2). Прежний
  чек 14d «в покое в воде B видит SWIMMING» заменён на эти два (он противоречил
  правилу 14e). Полная сетка G1..G7 на нашем сервере не гонялась — предобпечённой
  геометрии/`/fill` на нашем сервере нет (см. ниже).
- **Gap (п.7):** полная сетка G1..G7 на нашем сервере **не гонялась**. Наш S10
  (`-Target ours`) честно фиксирует «нет `/fill`» — построить 16 геометрий в
  мире прогона нечем. Живой S8 использует один пруд из шаблона v3
  (`-OursSpawnOverride 106,41,-31`). Чтобы прогнать сетку, нужен либо готовый
  мир с геометрией (NBT-копия или будущий `ChunkStore`-API), либо dev-команда
  установки блоков; шаблон v3 при этом не трогается. Оставлено как долг.
  **Закрыто на шаге 15 п.2/п.3:** dev-параметр `dev_geometry` применяет команды
  стенда к in-memory store, сетка прогнана (`s10-ours-r1`, 98.7 %).

## Шаг 15: закрытие плавания — метрика в тиках, геометрия, waterlogged, ползание (16.11.2026)

Всё — black-box: правила только из ванильных трейсов и дифференциальных прогонов,
чужой код не читался. Инструменты: `tools/trace-analyze/s10-compare.py`,
`anvil-block.py`, `s10-swim-table.py`; стенд — `tools/testclient/run-scenario.ps1`.

- **п.1 sprint от `player_command`, метрика в тиках (коммит `b5b96d8`).** Прежняя
  метрика считала только `set_entity_data`-переходы; sprint оказался потоком
  `player_command` start/stop, не битом `player_input` (0x40). Реализовано:
  `MoveSlot::store_command`/`take_command`, `Player::apply_command` (после
  `apply_input`). Oracle в 50 мс-тиках по r3: **точно 4605/4633 (99.4 %), в
  пределах ±1 тика 4629/4633 (99.9 %)**; переходы — 52/68 точно, 68/68 в ±1.
  4 остатка beyond ±1 — сетевой лаг 1–2 тика на краю.
- **п.2 dev-параметр геометрии (коммит `2d5a339`, BOM-фикс `ff62fcb`).**
  `dev_geometry = "файл"` в конфиге: `/setblock` и `/fill` применяются к
  in-memory `ChunkStore` перед bind, мир на диске не пишется; warn при
  включении. Имя+состояние → id по `blocks.json`; неизвестное — ошибка с
  номером строки. Шаблон v3 не трогается.
- **п.3 прогон G1..G6 (коммит `9388ff7`, сверитель `5c3e4b5`).** Наш сервер
  `s10-ours-r1` — 16/16 PASS, ваниль `s10-vanilla-r5` — 16/16; сверка одним
  кодом (`s10-compare.py`): **732/742 поз и флагов совпадают (98.7 %)**.
  10 расхождений — только G4-wl (см. §2.2 `docs/SWIMMING.md`).
- **п.4 waterlogged.** 10 расхождений доказаны разбором Anvil-мира
  (`anvil-block.py`): G4-wl-slab-bottom — ваниль прогнала обновление жидкости и
  вынесла воду на поверхность платформы, наш статический store оставляет air +
  ваниль сняла sprint (наш 0x08); G4-wl-fence — ваниль не включает присед, когда
  тело в клетке коллизии забора (наш 0x02). Оба — известные долги: нет симуляции
  жидкости и нет модели коллизий. Гипотез не выдвигалось: показаны позы обеих
  сторон. Полный разбор — §2.2 `docs/SWIMMING.md`.
- **п.5 ползание (коммит с исходниками).** Правило (S11): поза форсируется по
  свободному месту над **текущей** позицией — < 1.5 → pose 3 `SWIMMING`/ползание
  (flags 0x00, hitbox 0.6), 1.5..1.8 → pose 5 `CROUCHING` (0x02, 1.5), ≥ 1.8 →
  входная поза (0). Данные о формах блоков нет в `blocks.json`, поэтому тест-клиент
  командой `shapes` пишет `reports/collision.json`
  (`BlockState.getCollisionShape(level,pos).min(Y)`, 35723 состояния / 30144
  непустых) — новый конфиг `collision_report`, `ChunkStore::free_headroom`,
  `Forced::from_headroom`, `pose_and_flags`. Oracle по S11 r2 — 112/112 сэмплов
  побитово. Live: `s11-ours-r1` (dev_geometry) против ванильного r2 —
  **94/94 выровненных сэмпла совпали** (у нас нет `block-above-dynamic`: нет
  рантайм-постановки блоков). Ходьба в низкий проём позу не понижает — упор в
  край делает клиент, модели коллизий у нас нет. Полный разбор — §5
  `docs/SWIMMING.md`.
- **п.6 не делали:** воздух, утопление, звук всплесков, подводность глаз — долг.

## Живой тест двух клиентов 16.11.2026: P1–P4

Владелец прогнал два клиента 26.3 против одного сервера и сообщил четыре
проблемы. Black-box: только живые логи, трейсы и дифференциальные прогоны;
декомпилированный код не читался.

- **P2 — мерцание анимации плавания у поверхности (коммит `44881fa`).**
  Введён гистерезис: вход в плавание только при глазах под водой
  (`starts_swimming`, `STANDING_EYE_HEIGHT = 1.62`, `ChunkStore::point_in_water`),
  продолжение — по ногам (`is_swimming`). Данные S10 r3 три формы не различают
  (все 52/68 точно и 68/68 в ±1 события), живой тест выбрал гистерезис;
  статус против живой ванили — `[hypothesis]`. Oracle не изменился; смещение
  глаза при приседе (1.27) — TODO. Подробно — `docs/SWIMMING.md` §2.1.
- **P2 (рецидив) — фантомный зазор `1/9` в водном столбе (16.11).** После
  гистерезиса владелец всё ещё видел прерывание анимации у поверхности. Живой
  репро (`E:\MineRust-data\live-repro-p2`, бассейн 10 в глубину, B всплывал с
  up+sprint+jump): серверный трейс (`MINECRAFT_SESSION_TRACE`, `session.json`)
  показал щелчки `24 ↔ 08` четыре раза за ~1 с ровно на проходе дробной части
  `y` ног через `0.888`. Причина: `ChunkStore::point_in_water`/`feet_in_water`
  считали верхнюю `1/9` клетки воды сухой (поверхность источника `8/9`) даже
  когда блок выше — вода; в сплошном столбе на каждой границе блоков был
  фантомный зазор. Фикс: вода в клетке с водой сверху занимает всю клетку;
  одиночный источник с воздухом сверху по-прежнему `8/9`. Тест
  `a_water_block_under_water_fills_its_cell` (падал до фикса). Живая
  перепроверка: щелчков `4 → 1`, наблюдатель A видит `54 → 2` смены состояния.
  S10-оракул не затронут (читает булевы фикстуры, не store). Подробно —
  `docs/SWIMMING.md` §2.1.
- **P3 — ползущий игрок выглядел стоящим у наблюдателя (коммит `7f56874`).**
  Причина операционная: в локальном `server.toml` владельца не было
  `collision_report`, сервер писал
  `WARN no collision_report: the forced crouch/crawl ... is disabled` и не
  форсировал позу. Поле добавлено в локальный конфиг (он не коммитится),
  `tools/run-server.bat` подсказывает его в тексте про отсутствующий конфиг.
- **P4 — запуск двух разных клиентов (коммит `2bc7646`).**
  `tools/run-two-clients.bat`: два Fabric-клиента (Alice/Bob) с отдельными
  `gameDir` и control-портами 25571/25572, адрес первым аргументом. Проверено
  запуском: оба клиента входят и видят друг друга.
- **P1 — ошибка клиента `EncoderException: Pipeline has no outbound protocol
  configured, can't process packet ServerboundChunkBatchReceivedPacket` при
  почти одновременном входе — НЕ ПОВТОРЕНА.** Контролируемый репро:
  release-сервер на `25566` из `E:\MineRust-data\live-repro` (с
  `collision_report`), два Fabric-клиента через `run-two-clients.bat`.
  Три попытки: входы с разницей 1.16 с; 0.30 с (quick play); **30 мс** через
  control-команду `connect` (две сокет-сессии, обе строки запроса подряд).
  Во всех трёх: оба `player joined`, по 473 чанка, оба видят друг друга
  (`crosshairEntity: RemotePlayer`), в серверном логе нет ни киков, ни
  timeout, ни ошибок соединений. Логов неудачного прогона на машине нет:
  консоль сервера владельца не перенаправлялась в файл, а клиентские
  `latest.log`/архивы `2026-11-16-*` такой ошибки не содержат.
  Побочно найден и исправлен баг инструмента (коммит `6017928`): control-команда
  `connect` передавала ненулевой `TransferState`, клиент слал handshake
  intention 3 (transfer), и наш сервер закрывал соединение
  (`transfer state not implemented`). Теперь передаётся `null` — обычный вход,
  проверено живым входом двух клиентов с разницей 30 мс.
  Гипотезы (не подтверждены): ошибка клиента — симптом закрытия соединения
  посреди чанк-батча (клиент при этом пишет
  `Negative index in crash report handler (18/22)` и процесс завершается), а
  закрыть его мог кик/keep-alive timeout — оба пишутся в серверный лог, поэтому
  для закрытия нужен вывод консоли сервера неудачного прогона. TODO: попросить
  владельца сохранить консоль сервера (или перезапустить с
  `RUST_LOG=net=debug` и перенаправлением stdout) и клиентский `latest.log`
  до перезапуска клиента. Для этого добавлен `tools/run-server-logged.bat`:
  он запускает сервер и дублирует консоль в `logs\server-<дата>.log` (UTF-8;
  по умолчанию `RUST_LOG=net=info,server=info`, `net=debug` включает пакеты).

## Остаточные факты и блокеры

- **Шаг 10 C:** punch→swing не детерминирован один-к-одному на S3 (22 punch, 11 swing); правила drain/aggregation и удержания на блоке не установлены. Нужны новые подтверждённые трейсы/семантика перед реализацией.
- **Шаг 10 live acceptance (закрыт):** S5 `2026-11-15_step10-live-ours-s5-observe` — wire и клиентские checks PASS: `player_input` `20 00 01 41 01 00`, metadata `02 → 00 → 08 → 00`, отдельные pose metadata `05 → 00`, sprint-on/off attributes; B observe подтвердил sneak `CROUCHING/true` и sprint `STANDING/false` (по одной попытке). S7 `2026-11-15_step10-live-ours-s7-retry2` — PASS: B действительно наблюдал A как `CROUCHING`, `isCrouching=true` при held-sneak; для held-sprint — `STANDING`, `false`; обе A/B пары трассированы. Оба S7 pose-наблюдения теперь имеют двухпопыточную клиентскую проверку.
- Spawn sprint не подтверждён: S7 held-sprint join содержит initial `update_attributes`, но причину значений из пакетов установить нельзя; spawn-код не утверждает sprint snapshot. В vanilla traces порядок атрибутов различается между C1/C2/C4 (26,13,8 / 8,26,13 / 26,8,13); сравнение по порядку снято — см. `update_attributes` ниже.
- S7 A-trace: `entity_event` seq 51 (`00 00 00 2f 18`), `player_position` seq 55 (`01 40 58 20 … c0 24 99 99 9a …`), до `set_health` seq 160 (`41 7a aa ab 11 00 00 00 00`, health 15.666667, food 17, saturation 0); `damage_event` до входа B нет (подробный аудит — раздел «S7: аудит здоровья A» выше). Пакеты не объясняют причину health; она **закрыта на шаге 12** (сохранённое `Health` ванильного мира, вход на `y=128` = `Pos.y` из `.dat_old`; не расхождение нашего сервера). S3 остаётся только диагностикой: 22 A `punch`, 11 B `swing_animation`; серии 4920–4922 и 5094–5096 следуют за `block_destruction`, но связи и правила агрегации не установлены, поэтому C не реализуется.
- **Остатки сверки (в) Шага 6.3:** вопрос порядка `update_attributes` **закрыт** — сравнение идёт как набор записей `(key, value, modifiers)` и в unit-тестах, и в parity (`crates/protocol/src/packets/entity_meta.rs::records_match`, `crates/parity/src/diff.rs`; решение — `docs/DECISIONS.md`, «Порядок записей `update_attributes` не является критерием parity»). Остаются: принять ли эвристику `entity_position_sync`; какое правило `rotate_head` (C2 и C4 противоречат); как учитывать джиттер сэмплинга. До решений точное побайтовое совпадение C2/C4 недостижимо по этим классам, но не из-за порядка атрибутов. Шаг 7.3 список не сузил (только вывел из него game mode).
- **Долг: три `player_info_update` на входе (Шаг 6.1, п. 8).** Шаг 7.3 закрыл
  запись о самом игроке; вопрос про self-записи (`ff00` пустой + запись 36 Б +
  латентностная) требует отдельного трейса с обеих сторон и не закрыт.
- **Опоздания тика в живом логе (Шаг 7.4).** При `players=0` опозданий нет
  (400 тиков, `late_max_ms = 0`). Нагруженная машина владельца (OBS, два
  клиента-JVM, Discord) даёт пачки `tick woke late`: живой лог 16.11 (tick
  2457, `late_ms=212.5`, `uptime_ms=134878`, `players=2`) с фазовой строкой
  `wake_late_ms=212.5`, `phases_total_us=63` — опоздание целиком в пробуждении
  (фазы в микросекундах, work ≈ 0.1 мс), затем 4 тика догона в пределах 0.3 мс
  (212 < `reset_after`·period = 250 мс, поэтому дедлайн не сбрасывается, а
  пропущенные слоты проигрываются). Это вытеснение потока ОС, не медленный тик.
  Мера A3/D7 (`tick_thread_priority`) теперь прокинута в `server.toml`
  (`crates/server/src/config.rs`, default off): A3 под конкуренцией за CPU
  убирает опоздания (late_ticks 27/4/10 → 1/0/0). Включать на нагруженной
  машине.
- **P1: ошибка клиента при почти одновременном входе (16.11).** Не повторена
  тремя сценариями (1.16 с / 0.30 с / 30 мс между входами); сервер не кикал и
  не таймаутил, оба клиента загрузили по 473 чанка и видят друг друга. Нужны
  консоль сервера неудачного прогона и клиентский `latest.log` до перезапуска —
  см. раздел «Живой тест двух клиентов 16.11.2026: P1–P4».
- (нет открытых) — Шаг 3.5 (p99 при uniform P=200 = 7.9–8.3 мс > 5 мс) закрыт
  `919fd75`/`9b6e810`; повторный замер 2.81–2.98 мс ≤ 5 мс (детали —
  `docs/BENCH-M4.md` §9 и «Шаг 3.5 — пересдача» выше).

## Побочные фиксы (инструменты)

- **[12.11.2026] Плагин Graphify для OpenCode падал при загрузке — исправлен.**
  OpenCode 1.17.13 грузит плагины двумя загрузчиками: серверный ждёт V2-объект
  (`export default { id, setup|effect }`), CLI-путь — V1-объект с `server()`
  (документированная схема «V1 + V2»). Старый `graphify.js` экспортировал
  `export const GraphifyPlugin = ...` (ни того, ни другого) — в логе сервера
  `Plugin must export a default definition with an id and an effect or setup
  function`, в CLI-пути `... must default export an object with server()`.
  Дополнительно `.opencode/opencode.json` регистрировал тот же файл как
  npm-пакет (`.opencode/plugins/graphify.js` → `NpmInstallFailedError`), а
  случайная запись `list` (от `opencode plugin list`) давала ещё один сбой.
  Теперь `graphify.js` — единый объект с `id`+`setup` (V2) и `server()` (V1);
  напоминание о графе регистрируется через `ctx.tool.hook("execute.before", …)`
  там, где такой хук есть. В 1.17.13 серверный V2-контекст tool/shell-хука не
  содержит (проверено по `@opencode-ai/plugin@1.17.13`), поэтому на этой версии
  напоминание фактически не срабатывает; оно включится на версиях с `ctx.tool`.
  Конфиг сведён к одному авто-обнаруженному плагину; самовольные правки
  `server.toml` не затрагивались.

## Шаг 16: закрытие плавания/ползания, затем трещины и серверный таймер копания (16.11.2026)

Плюс отдельный побочный фикс `tick_thread_priority` (см. ниже, раздел «Опоздания
тика»).

- **16.0a — `collision.json`/`E:\MineRust-data` не коммитятся; сервер без
  `collision_report` работает.** `git ls-files` по `collision|reports/|hardness|
  MineRust-data` даёт единственную строку `crates/data/src/collision.rs`
  (загрузчик, не данные); ни `reports/`, ни `collision.json`, ни файлов из
  `E:\MineRust-data` в индексе нет — этот каталог вне репозитория
  (`git rev-parse --show-toplevel` = `E:/projects/MineRust`). Загрузка
  коллизий вынесена в `load_collisions` (`crates/server/src/lib.rs`);
  отсутствие настройки даёт пустой `CollisionShapes` (тогда
  `ChunkStore::free_headroom` = `None`, `Forced::from_headroom(None)` =
  `Forced::None` — ползание выключено) и `warn` с именем настройки и правила.
  Тест `a_server_without_the_collision_report_disables_the_crawl_rule_and_warns`
  ловит `tracing`-строку на этом же потоке (свой `Sink`-writer, без новых
  зависимостей) и прогоняет `build_world` на минимальном сейве — старт
  принимает отсутствующий отчёт. `cargo test -p server`: 46 passed.

- **16.0b — тест (в) с `--ignored` на нашем сервере против ванильных трейсов.**
  `cargo test -p parity --release --test entity_stream_parity -- --ignored
  --nocapture` (135.8 с): **1 failed** (тест красный диагностический, его не
  меняли). Отчёт `C:\MineRust-data\traces\entity-stream-parity.md`, 286 строк
  расхождений:

  | Сцена | Красных | Причина |
  |---|---|---|
  | С1 (вход, спектатор 3) | 1 | `update_attributes` о B: у нас первым идёт key `8`, у ванили key `26` — расхождение только **порядка** атрибутов (известное, Шаг 11: порядок самого ванильного трейса непостоянен); всё остальное байт-в-байт |
  | С2 (ходьба) | 173 = 1 счёт + 142 тип + 30 байт | у нас 178 пакетов о B, у ванили 176. Сравнение идёт **по номеру пакета**, поэтому одно расхождение сдвигает всё дальше: ваниль шлёт `entity_position_sync` о игроке, у нас его нет, плюс другой темп/байты `rotate_head`/`move_entity_pos_rot` (причины Шага 4). 30 байтовых расхождений (позиции/углы) — следствие того же сдвига, а не отдельные механизмы |
  | С4 (поворот на месте) | 100 = 1 счёт + 89 тип + 10 байт | у нас 111, у ванили 113: тот же сдвиг в потоке поворота; байты `rotate_head` (угол головы) сходятся по смыслу, но не по номеру пакета |
  | С5 (вход, survival 0) | 6 = 1 счёт + 5 тип | у нас 11, у ванили 12: ваниль шлёт `entity_position_sync` о B при входе, у нас нет; из-за этого хвост (`remove_entities`/`player_info_remove`) съезжает на один пакет |
  | С6 (вход, creative 1) | 6 | то же, что С5 |

  Общее число проверок, которые прошли (не входят в отчёт): совпали сами
  факты видимости — `set_entity_data` о B ×2 в каждой сцене, скрытность
  (`invisibility 0x20`) в спектаторе (С1/С2/С4) и видимость в survival/creative
  (С5/С6). Тест (в) — красный **диагностический**, gate его не задевает
  (`ignored`).

- **16.0c — готовые dev_geometry-сцены (`tools/testclient/geometry/`).**
  Три файла с README, ни один не включён по умолчанию: `pond.txt` (9×9 бассейн,
  поверхность y=41, `spawn_override = "106,41,-31"`), `crawl.txt` (проёмы 1.0 и
  1.5 над платформой y=201, `spawn_override = "56,201,-47"`; геометрия — та же,
  что в S10 G7/S11), `waterlogged.txt` (вода на платформе + waterlogged
  slab-top/slab-bottom/stairs/fence, `spawn_override = "92,201,-32"`).
  `README.md` даёт строки конфига, команду `observe` и таблицы ожидаемого
  поведения B из ванильных прогонов (`docs/SWIMMING.md` §2.1/§2.2/§5), включая
  два известных долга (нет симуляции жидкости → `G4-wl-slab-bottom`; нет модели
  коллизий → `G4-wl-fence`). Тест `shipped_dev_geometry_files_parse`
  (`crates/server/src/geometry.rs`) прогоняет `load()` по каждому `.txt` этого
  каталога — опечатка валит `cargo test -p server` (47 passed).

- **16.0d — долги механик плавания/ползания сведены в один список.**
  `docs/HANDOFF.md` получил актуальную секцию Шага 16 и таблицу долгов;
  `docs/MECHANICS.md` — раздел «Долги механик плавания/ползания» и запись в
  журнале. Долги: симуляция жидкости, модель коллизий, воздух (`air supply`),
  утопление, звук всплеска, подводность глаз — все с указанием, есть ли данные
  и где раздел `docs/SWIMMING.md`.

- **16a.1 — экспорт твёрдости мод-командой + серверная настройка
  `hardness_report`.** В `tools/testclient/mod` добавлена команда `hardness`
  (`HardnessReportCommand.java`, зарегистрирована в `TestClientMod`): для
  каждого состояния пишет `hardness` = `BlockState.getDestroySpeed(level,pos)`
  (твёрдость; `< 0` — неразрушимо), `handSpeed` =
  `Player.getDestroySpeed(state)` (скорость держащим пустую руку) и список
  `unbreakable` — в `reports/hardness.json` вне репозитория. Только публичный
  API; мод скомпилирован `javac` против named-jar 26.3. В `crates/data` —
  загрузчик `BlockHardness` (`hardness.rs`) + варианты `DataError`; сервер
  читает файл настройкой `hardness_report` (как `collision_report`) через
  `load_hardness`: без неё `GameConfig.hardness = None` и `warn` («таймер
  копания доверяет клиенту»), с битым путём — отказ старта. Тесты: 5 в
  `data::hardness`, 1 в `config` (настройка опциональна/валидируется), 1 в
  `server::lib` (отсутствие + битый путь). `cargo test -p data -p server -p net`
  зелёный.

- **16a.2 — модель стадий/времени выведена из трейсов и подтверждена
  `hardness.json` (закрыто).**
  Скан всех `autotests\*\mcproxy\*.jsonl` за 14.11.2026 по `block_destruction` +
  `level_event` 2001 дал **5 прогонов с реальным копанием** (1303, 1451, 1606,
  1636, 1644), 11 сломанных блоков и 6 прерванных копаний; блоки определены по
  `data` события 2001 и `reports/blocks.json`. Полная таблица —
  `docs/BLOCK-BREAKING.md` §1.7. Итоги:

  | Блок (state) | Стадии | Пакетов | мс | ≈тиков |
  |---|---|---|---|---|
  | dark_oak_leaves (438/442) | 1,3,5,6,8,10,11 | 8 | 299 | 6 |
  | oak_leaves (274/278) | 1,3,5,6,8,10,11 | 8 | 300 | 6 |
  | oak_leaves (274) | 1,3,5,6,8,10,11,**13** | 9 | 352 | 7 |
  | dirt (10) | 0..10 | 12 | 751 | 15 |
  | grass_block (9) | 0..9 | 11 | 796/807 | 16 |
  | dark_oak_log (158) | 0..10 (шаг ≈6 тиков) | 12 | 2999 | 60 |

  **Исправлена ошибка плана:** запись «stage 0..9, финал `0xFF`» неполная — в
  трейсах есть 10, 11 и 13 (все от одного копающего, entityId 826/51), а `0xFF`
  — это **сброс**, который приходит и при прерывании (2001 и `block_update`
  нет). Форма последовательности — `stage(k) = floor(10·d·k)` при постоянном
  `d` (листва = `floor(1.67·k)`, бревно = `floor(0.164·k)`), т.е. `stage` =
  `floor(progress·10)` **без ограничения сверху**, а шлётся только при смене
  значения. У листвы прогресс доходит до ≈1.3 и стадии 11/13 приходят **уже
  после** 1.0, а `block_update` в воздух — только после клиентского
  `player_action 0x03`: сервер **не ломает блок сам** по прогрессу.
  Проверка гипотез владельца — `docs/BLOCK-BREAKING.md` §5 п.7 (4 подтверждены,
  2 опровергнуты). Наблюдаемый прогресс в момент сброса 0.94...1.36 — это и
  есть допуск проверки времени (вопрос §6 п.8).

  **Абсолютная шкала закрыта:** `reports/hardness.json` сгенерирован командой
  `hardness` на подключённом тест-клиенте (наш сервер + Fabric-клиент Bob):
  **35723 состояния, 116 несокрушимых** (`hardness < 0`), **2863 с
  `hardness = 0`** (мгновенные), `handSpeed = 1.0` у всех. Значения: листва
  0.2, dirt 0.5, grass_block 0.6, dark_oak_log 2.0, stone 1.5.
  `T = 30·hardness/speed` совпало **точно** для листвы (6 тиков), dirt (15) и
  бревна (60); grass даёт 18 по формуле против 16 в трейсах — расхождение в
  2 тика вынесено в 16a.4 и там закрыто: это сдвиг доставки первого пакета
  (§1.8, оракул §1.7.1). Попутно исправлена команда:
  первый экспорт дал `handSpeed = 0.2` (клиент ещё падал после входа — игра
  делит скорость копания на 5 в воздухе), теперь `hardness` отказывается
  работать не на земле; файл перегенерирован. Допуск проверки времени (§6 п.8)
  выбран мягкий — (а) «не раньше чем за 3 тика до расчётного T», помечается
  гипотезой; переключается настройкой.

- **16a.3 — серверный таймер копания и стадии `block_destruction` (закрыто).**
  `BlockDestruction` (id `0x05`) в `crates/protocol` — `entityId varint +
  packed pos + stage u8`, `RESET = 0xFF`; побайтный тест по четырём payload'ам
  из трейсов (§1.3). `ServerMessage::BlockDestruction`, кодирование в
  `connection/play`, `DigState` у `Player` (одно копание на игрока), новый
  `game/dig.rs`: за тик `progress += step` (**f32**, иначе шесть шагов `1/6` не
  дают наблюдаемую стадию 10), `stage = floor(progress·10)` без клиппинга,
  пакет только при смене байта; рассылка **всем, кроме копающего**. Таймер
  идёт после `scheduler.tick` (игрок, сменивший регион, продвигается в новом).
  `block_action` переписан: start/abort/finish/refuse/stop, creative —
  по-прежнему мгновенный; на finish `0xFF` → `level_event` 2001 → `block_update`
  → ack (порядок трейса); проверка времени `progress + t·step >= 1.0`,
  `t = GameConfig.dig_tolerance_ticks = Some(3)` (гипотеза, §6 п.8), `None` —
  без проверки; отказ возвращает реальное состояние копающему, наблюдателям
  только `0xFF`. `hardness = 0` мгновенно, `< 0` неломаемо (finish
  отклоняется), без отчёта / без состояния — прежнее доверие клиенту.
  `ToGame::Quit` сбрасывает трещины копания. Тесты: 18 в
  `game::block_action` (в т.ч. traced-последовательность листвы
  `1,3,5,6,8,10,11`, dirt с 0, отказ и откат, допуск, unbreakable, instant,
  quit) + стадии `dig::stage`; `cargo test --workspace` зелёный.

- **16a.4 — оракул интервалов, живой A/B, разбор grass и 2019/2020 (закрыто).**
  Оракул (`crates/net/src/game/dig_oracle_tests.rs`, 5 тестов): 10 ванильных
  копаний §1.7 перечитаны как `(стадия, тик)`; модель — стадия 0 на тике 1,
  `k ≥ 1` на `ceil(k·T/10)`, `T = 30·hardness`; на каждое копание один
  постоянный сдвиг доставки (медиана остатков), после него каждая стадия,
  каждый переход и span должны быть в ±1 тике. **9 из 10 копаний в ±1**
  (худшее 0.85 тика), 1644 — 2.33 тика (пачка доставки; тест
  `the_1644_grass_dig_flags_its_delivery_stall` требует отклонение > 2, не
  прячет его). Сдвиг у всех 1.47...1.85 тика — это и объясняет «16 тиков» у
  grass: `T = 18` верна, §1.8 (в обоих прогонах `on_ground = true` до и после,
  смены предмета нет, штрафы ×5/÷100 исключены). Допуск: `finish − T` =
  −1.93...+0.41; N = 0 отклонил бы 5 копаний, N = 1 — 2, N = 2 и 3 — 0;
  оставлено **3** (запас 1.07 тика), §6 п.8.

  Состояния (п.3b) и live A/B (п.4), прогоны `2026-11-17_0142_s12-ours` и
  `_0149` (оба 22/22, 10 сломанных блоков): вода под ногами с сухими глазами
  не замедляет — dirt 14 тиков при T = 15; камень в яме под водой (игрок
  сначала тонет с `on_ground = false`, затем стоит с глазами в воде) не
  сломан за 481 тик при 151 на земле — ≥3.2×, согласуется с ×5, завершённого
  копания нет. Присед совпадает с землёй (17/15/151/6); **камень пустой
  рукой клиент копает 151 тик при модели 45** (штраф «нет инструмента»),
  поэтому стадии B идут 0..33 все 151 тик — полная стадия к ~44 тикам вместо
  ~135, долг. Воздушная стойка в обоих прогонах приземляется до старта
  (прицеливание ~250 мс дольше падения), live-воздух не проверен; финиш
  3.09 серверных тика при T = 6 принят на границе допуска. Отказов в этих
  прогонах нет; но в `_0131` первое копание заняло 2.65 серверных тика
  (клиент догнал 11 тиков за 1.4 мс) и было отклонено — честное копание
  откатилось (п.3c), обход стенда и `dig_tolerance_ticks = None` описаны в
  §6 п.8. Байты 2019/2020: в 1644 их получили **оба** соединения (12×2019 +
  4×2020), 2001 — только наблюдатель; комментарий `LevelEvent` исправлен,
  реализации в 16a.4 нет (16a.5). Раннер: выбор трейсов по содержимому
  (`Select-TraceByPattern`), `[int]` при разборе `level_event` (иначе 2001
  читался как 209), пауза после прицеливания на кейс.

## Шаг 17: трещины включаются, делитель камня, порог ванильного finish, чек-лист владельца (17.11.2026)

Причина, по которой владелец «не видел трещин», установлена на 17.0: в его
рабочем `server.toml` не было `hardness_report`. Без отчёта серверный таймер
выключен, `block_destruction` не шлётся, а блок всё равно ломается по finish
клиента. Дальше — пять подшагов, каждый отдельным коммитом.

### 17.0 — отчёты в конфиге и одно предупреждение на старте

- `server.example.toml`: у `collision_report` и `hardness_report` описано, что
  именно отключается без файла (КАПСОМ), команды генерации (`shapes`,
  `hardness`), ожидаемые числа (35723 состояния, 30144 непустых коллизий,
  116 неразрушимых) и что путь задаётся в своём `server.toml`.
- `crates/server/src/lib.rs`: предупреждение ровно одно на старт, с
  подсказкой: `set collision_report=<path>, generate the file with the test
  client's `shapes` command` / то же для `hardness_report`/`hardness`. Тесты
  проверяют текст, что имя настройки и команда в нём есть и что вхождение
  ровно одно (не на чанк и не на тик). `cargo test -p server` = 49 passed.
- Рабочий `server.toml` (вне git) указывает на
  `E:\MineRust-data\reports\collision.json` и `hardness.json`.
- Проверка: release-сервер грузит оба отчёта (`collision report loaded
  states=30144`, `hardness report loaded states=35723`), предупреждений о
  пропуске нет.
- Повтор S12 на бинарнике до 17a (`2026-11-17_1720_s12-ours`, с
  `-HardnessReport`): **22/22**, B получает стадии, сбросы и эффект
  ломания — включение отчёта возвращает трещины без изменения кода.

### 17a — делитель 100 у блоков, которым нужен инструмент

- `hardness.json` теперь несёт `toolRequired` (список id состояний с
  `requiresCorrectToolForDrops()`): **18035 из 35723**, ни одного
  флагованного без данных о твёрдости, 52 флагованных неразрушимых. Живой
  экспорт командой `hardness` на клиенте (файл вне репозитория):
  `states=35723, unbreakable=116, toolRequired=18035`.
- Данные по умолчанию-состояниям: `stone` 1.5 флагован, `iron_ore` 3.0
  флагован, `cobblestone` 2.0 флагован, `obsidian` 50.0 флагован;
  `dirt` 0.5, `grass_block` 0.6, `oak_leaves` 0.2, `oak_log` 2.0 — без
  флага; `bedrock` −1.0 без флага (неразрушим).
- Модель: `step = speed / hardness / (requires_tool ? 100 : 30)`; пустая рука
  никогда не «правильный инструмент», поэтому флаг и есть переключатель.
  `T = 30·h` для листвы/dirt/grass/бревна и `T = 100·h` для камня/руды.
  Проверка по живому клиенту (A — время `player_action 0x00 → 0x03`):
  листва 6.0 при T=6, dirt 14.7 (15), grass 16.7–17.4 (18), бревно 60 (60),
  **камень 151.0 при T=150** (старая модель /30 давала 45 — отсюда «трещины
  полные к 44 тикам»).
- Код: `BlockHardness::requires_tool/tool_required_len` (+ юнит-тест, старый
  отчёт без списка грузится и не флагует), `TOOL_REQUIRED_DIVISOR = 100.0` в
  `crates/net/src/game/dig.rs`, выбор делителя в `step_per_tick`
  (`block_action.rs`). Байтовые/тиковые тесты:
  `a_tool_required_block_digs_at_the_hundred_divisor` (стадии 0..10 на
  `15k ±1`, стадия 10 на 151) и
  `a_stone_finish_at_the_thirty_divisor_time_is_refused`. `cargo test -p data
  -p net` = 41 + 193 passed.
- Live A/B `2026-11-17_1745_s12-ours`: **22/22**; камень (земля и присед):
  стадия 0 на старте, 1 на 14-м тике, далее каждые 15 (2/30 … 9/135, 10/150,
  `0xFF` на 151); стадий у B 48 вместо 82/81 (вода 45 вместо 120). Время
  копания у клиента не изменилось (grass 16.7/17.4, dirt 14.7, leaves 6.0,
  stone 151.0); допуск 3 тика не менялся.
- Границы: моделируется только **пустая рука**. Инструменты, зачарования и
  их скорости — отдельный шаг (17a.4).

### 17b — ванильный приём finish: проверка есть, но не доля прогресса

Диагностическая команда `digpacket {k, start?}` тест-клиента (**только 17b,
сценариями не используется**) шлёт на ванильный сервер 26.3 (offline,
survival, `playerGameType=0`, `instabuild=0`, пустая рука) start
(`player_action` 0) по блоку под прицелом и finish (`player_action` 3) через
`k` клиентских тиков для той же позиции и грани; позицию, грань и
последовательности даёт клиент. Путь через `MultiPlayerGameMode` доказанно
негоден: `destroyBlock(pos)` и `stopDestroyBlock()` шлют action **2 (abort)**,
и 8 прогонов первой версии команды не сломали ничего (в трейсе `[2, ...]`).

Один стенд, один игрок, одна позиция блока (100,200,−34):

- **принят на `k = 1`**: dirt (T = 15), stone (150), cobblestone (200),
  iron_block (500), ender_chest (2250); листва/dirt/grass приняты на своих
  наборах `k` (минимальный прогресс 17–44 %);
- **отклонён при `k ≤ 3`**: ancient_debris (3000), obsidian (5000),
  bedrock (−1, неразрушим);
- **finish без start**: отказ, 0 стадий (`_nostart2`);
- ни один опыт не отключил клиента — крафт-пакеты ваниль принимает без
  разрыва;
- аномалия: последняя стадия перед ломанием аномально велика (stone
  33/39/44, листва 73, dirt 230) против обычных 0..13 у копаний клавишей
  (1644) — вероятно унаследованный аккумулятор, **не установлено**.

Вывод: ваниль finish **проверяет**, но это **не доля прогресса** — stone и
cobblestone ломаются на 1-м тике (0.7 % и 0.5 % от T), а граница приёма лежит
**между T = 2250 (принят) и T = 3000 (отказ)**, то есть между hardness 22.5 и
30. Правило не установлено, поэтому **порогом допуск не заменён**: остаётся
`dig_tolerance_ticks = Some(3)` («не меньше `T − 3` тиков»), а расхождение
описано в `docs/BLOCK-BREAKING.md` §1.11 и §6 п.9. Полный разбор и три
помеченные гипотезы — там же; стадии, `0xFF` и «сервер не ломает блок по
прогрессу» не меняются. `_0131` (честное копание 2.65 серверных тика при
T = 6 отклонено) объясняется нашим допуском, а не ванильным поведением.

### 17c — чек-лист владельца (геометрия и копание)

`tools/testclient/geometry/README.md`: добавлен раздел «Как включить» (оба
отчёта, точные строки предупреждений, `shapes`/`hardness`, ожидаемые числа,
«в creative трещин нет») и чек-лист владельца на один заход с готовыми
конфигами и ожиданием у наблюдателя B: пруд (`pond.txt`), ползание в проёме
1.0 (`crawl.txt`) и 1.5, waterlogged (`waterlogged.txt`), копание
листвы/dirt/grass/камня (камень ≈151 тик после 17a), копание в прыжке и в
воде. Там же известные долги: нет трещин в creative, моделируется только
пустая рука, копающий не видит серверных стадий, finish в воздухе/воде
доверяется без проверки времени.

### 17d — playerdata: план, кода нет

`docs/PLAYERDATA.md` — план сохранения позиции/поворота/здоровья:

- **Факты:** `players/data/<uuid>.dat` — gzip-NBT с именованным корнем;
  имя файла — не UUID из чата, первые три группы пишутся байтами в обратном
  порядке; в шаблоне единственный файл ParityBot (778 Б); ридер
  `world::read_world_document` есть, **writer'а дискового NBT нет**;
  `set_health` (clientbound id 106) не реализован; при входе ваниль шлёт
  `player_position` (поворот внутри) и `set_health` — S7 conn-1
  `(96.5,128,−10.3)` 15.666667/17/0, conn-2 спавн 20/20/5.
- **Формат:** вариант A — ванильный NBT (ридер есть, S1 проверяет именно
  `.dat`); B (свой JSON) и C (не сохранять) отвергнуты.
- **Папка:** отдельный `save_dir` (предложение
  `E:\MineRust-data\worlds\minecraft-v3`), **никогда** шаблон v3;
  канонизация путей и отказ на равенство/вложенность + тест.
- **Поля:** `Pos`, `Rotation`, `Health`, `foodLevel`,
  `foodSaturationLevel`, `playerGameType`, `Dimension`, `DataVersion` 5023;
  существующий файл обновляется по известным полям, незнакомые теги
  сохраняются.
- **Тайминг:** выход игрока, автосейв (6000 тиков), остановка сервера;
  запись через `<uuid>.dat.tmp` + rename; один владелец данных, без `Mutex`.
- **Связь с шагом 18:** playerdata не сохраняет мир; сломанные блоки и
  `dev_geometry` живут только в памяти до шага сохранения чанков.
- **Тесты:** round-trip дискового NBT, сохранение незнакомых тегов, проверка
  пути, валидация значений; интеграционные на вход/выход; харнесс `Invoke-S1`.
- **Размер:** средний; риски — запись в шаблон, UUID в offline/online,
  `DataVersion`, смерть/респавн не моделируются.
- **Вопросы владельцу (7):** папка, полнота NBT, health/food сейчас или
  после урона, источник `playerGameType`, набор триггеров автосейва,
  `stats`/`advancements` не пишем, смерть вне шага.

## Шаг 18: мгновенное ломание и водные блоки (17.11.2026)

Подшаг перед playerdata: закрыть правила «твёрдость 0» и «блок несёт воду» по
ванильному трейсу, до того как к playerdata добавятся сохранения. Метод —
чёрный ящик (наблюдение и трейсы, без кода ванили). Новый сценарий
`tools/testclient/scenarios/s14-instantbreak.json`: две стойки (удержание
`z = −34`, тап `z = −30`) и девять одноблочных ячеек по `x` на базе `y = 200`.

### п.1 — диагностика нашего сервера (S14 ours, `2026-11-17_s14-ours`)

Наш сервер уже ломал блок мгновенно (твёрдость 0 — отдельное правило 16a.3),
но замена всегда воздух и каскадов нет. Прогон снял факты для сверки: короткая
трава уходила в воздух, водные растения и waterlogged-полуплита/ступени **тоже**
в воздух (ваниль — в воду), `tall_seagrass`/`kelp` теряли одну клетку.

### п.2 — ванильный трейс (S14 vanilla, `2026-11-17_s14-vanilla`)

Тот же стенд и сценарий на ванили 26.3. Правила из трейсов
(`mcproxy\s14-samples.json` + попакетный разбор; полный разбор —
`docs/BLOCK-BREAKING.md` §1.12):

1. **Мгновенный блок ломается на start.** Твёрдость 0 → клиент шлёт **только**
   `player_action 0x00` (никогда `0x03`); сервер отвечает `block_update` замены,
   затем `block_changed_ack` с эхом sequence. Прежняя гипотеза «на start ack не
   шлётся» **опровергнута** (ack есть и он после `block_update`).
2. **Замена — по состоянию блока.** Блок, несущий воду (`waterlogged=true` или
   водное растение `seagrass`/`tall_seagrass`/`kelp`/`kelp_plant`), оставляет
   **воду** (`state 89`); наземные растения (short_grass, leaf_litter,
   dandelion) — воздух (`state 0`). `2001` несёт `data` = id сломанного
   состояния, не замены.
3. **Каскады.** `tall_seagrass` — двойное растение (ломание любой половины
   убирает обе: `2001` 2371 + 2372). `kelp`/`kelp_plant` — столбец (ломание
   части убирает весь келп выше: `2001` 18445/18424/18421). Остальное — одна
   клетка. Клиент при этом сам шлёт ещё start после перенацеливания (seagrass 2,
   tall_seagrass 3, kelp 3, плюс abort).
4. **Аудитория `2001`.** Прицельный блок — всем, **кроме** копающего;
   каскадная часть — всем, **включая** копающего (трейс: A получил `2001` для
   нижней половины `tall_seagrass` и для kelp y203, но не для блока под
   прицелом).

`2019/2020` (шум во время копания блоков твёрдости > 0) — как в §1.9, не наши.

### п.3 — реализация (Шаг 18) и её тесты

- Новый модуль `crates/net/src/game/break_block.rs`: `break_blocks` +
  `replacement` (через `BlockStates::water_surface` → `minecraft:water`) +
  `cascade` (`tall_seagrass` — вторая половина, `kelp`/`kelp_plant` — весь
  столбец выше) + `announce` (аудитория `2001`: прицельный — без копающего,
  каскадный — со всеми; `block_update` — всем).
- `crates/net/src/game/block_action.rs`: `start` ломает мгновенный блок
  (`is_instant`) без таймера и трещин; `deliver`/`AIR_STATE` — `pub(super)`.
- Тесты `crates/net/src/game/block_action_tests.rs`: мгновенный на start;
  waterlogged → вода; seagrass → вода; finish по воде отклонён; `tall_seagrass`
  обе половины (копающий видит `2001` каскада, не видит прицельного); столбец
  kelp; join после waterlogged; `cargo test -p net` — 199 passed.
- Побайтные тесты S14 в `crates/protocol/src/packets/play.rs`
  (`s14_instant_break_packets_roundtrip_the_trace`); `cargo test -p protocol` —
  139 passed.
- Сценарий и раннер: per-kind `rows`, 2-попыточная aim-петля, per-cell ack
  (`tools/testclient/run-scenario.ps1`, `Get-S14Commands`).

### п.4 — живой прогон ours против ванили

`2026-11-17_s14-ours-r2` и `-r3` (после правки аудитории `2001`) — **0 failed
checks**. Совпадение с ванилью по всем клеткам: наборы `2001` (и `data`),
замена (воздух/вода), каскады (`tall_seagrass` 2371→2372, kelp
18445→18424→18421/18445) и аудитория. Расхождения — только форма (см. долг) и
id нижнего kelp.

### Что дальше и долг

- **Дальше:** Шаг 18 playerdata (18.0/18a/18b) — план в `docs/PLAYERDATA.md`,
  вопросы владельцу там же.
- **Долг (Шаг 18):** `section_blocks_update` не реализован (у ванили замена при
  2+ изменённых блоках идёт этим батчем, у нас — `block_update` на каждую
  часть); каскады только у `tall_seagrass`/`kelp` (`tall_grass`, `large_fern`,
  цветы, `cave_vines`, `hanging_roots` не покрыты); ванильный shape-update
  `kelp_plant`→`kelp` на дне столбца не моделируем; `level_event 2019/2020` не
  шлём.
- **Блокеры:** нет.

### Открытый вопрос из Шага 17 (не закрыт)

Правило ванильного приёма `finish` не установлено (`docs/BLOCK-BREAKING.md`
§1.11, план §6 п.9): ваниль **проверяет** finish, но это не доля прогресса
(граница между T = 2250 и 3000). Наш допуск `T − 3` тика оставлен; вопрос
владельцу остаётся открытым.

## Шаг 18 (playerdata): 18.0, 18a, 18b (17.11.2026)

Этап закрыт по 18.0 и 18a; 18b — анализ, живой прогон S13 **не запускался**.

### 18.0 — хвосты Шага 17 (закрыто)

- **18.0.1 (закрыто).** `GameConfig.dig_tolerance_ticks` по умолчанию `None`:
  сервер доверяет любому `finish` клиента. `Some(n)` остаётся настройкой для
  стендов, где допуск нужен; решение помечено как «настройка владельца,
  временное». Стадии трещин у наблюдателей не меняются.
- **18.0.2 (закрыто, гипотеза опровергнута).** Гипотеза «ваниль принимает
  finish на ~70 % прогресса» не подтвердилась: порога в данных нет.
  Diagnostic `digpacket` расширен полями режима игры, предмета в руке и
  клиентской скорости копания. Живых прогонов пар «принят/отклонён» после
  расширения не было — **не установлено**.
- **18.0.3 (закрыто, факт трейса).** Аномальная последняя стадия в
  `digpacket`-трейсах — свойство самого ванильного трейса, наш сервер не
  затронут: `dig::stage` считается от свежего `DigState` и лежит в 0..13.
  Разбор — `docs/BLOCK-BREAKING.md` §1.11–1.12/§6.9, коммит `b8c7ade`.

### 18a — playerdata (закрыто)

**Решения владельца (7 вопросов, `docs/PLAYERDATA.md` §10).**
`save_dir = E:\MineRust-data\worlds\minecraft-v3` (папка на мир, внутри
`players/data/<uuid>.dat`; не шаблон, не внутри шаблона, не `vanilla\world`;
тесты и scenario-runner — отдельная папка). Существующий файл читается,
известные поля обновляются, незнакомые теги пишутся обратно; новый игрок —
минимальный компаунд; `DataVersion` — из `level.dat`. `Dimension` не
overworld → `warn` и спавн. Виталы — как в файле; новый 20/20/5;
`Health <= 0`/не число → 20 + `warn`. Режим: приоритет конфига,
`force_gamemode` (default `true`) — `false` отдаёт выбор файлу, если в нём
есть `playerGameType`. Триггеры: выход + автосейв (default 6000 тиков) +
остановка; запись не в тик-потоке. `players/stats` и `players/advancements`
не пишутся. Смерть/респавн — вне шага.

**Реализация.**
- `crates/protocol`: `SetHealth` (id 106; health f32 BE, food VarInt,
  saturation f32 BE — S7) и `NbtTag::encode_disk`/`decode_disk` (дисковая
  форма с именованным корнем). Коммит `55dfd0f`.
- `crates/playerdata` (новый крейт, 13 тестов): `Store::open/read/write`,
  `SavedPlayer`/`Snapshot`, `file_name` (реверс первых трёх групп UUID),
  атомарная запись `.tmp` + `flush` + `sync_all` + `rename`, защита пути
  (`ForbiddenPath` до создания папки и повторно после), очередь
  `sync_channel` + поток `playerdata-writer`, `Writer::shutdown(timeout)` с
  дренажом. Ошибка записи — `warn`, поток живёт. Коммит `147e4b5`.
- `crates/net`: `PlayerdataRuntime` в `GameConfig`/`GameHandle`; вход читает
  файл через `spawn_blocking`; `resolve_entry` (сохранённая поза только без
  `spawn_override` и только overworld) и `resolve_game_mode` — чистые
  функции; join шлёт `player_position` с сохранёнными позой и поворотом,
  затем `set_health` (7-й пакет); `Player` несёт uuid/health/food/saturation;
  `ToGame::Quit` и автосейв (`tick.is_multiple_of`) кладут снапшоты в
  очередь, финальный `save_all` — на остановке. Коммит `41c658c`.
- `crates/server`: `save_dir`, `force_gamemode`, `autosave_interval_ticks`;
  store и writer открываются **до** bind (нарушение пути или
  неоткрываемая папка — отказ старта); без `save_dir` сохранение выключено
  и одно `warn`; на остановке `writer.shutdown(5s)`. Коммит `9499d9d`.

**Тесты.** `playerdata` 13 (round-trip gzip-NBT, незнакомые теги, битый файл,
`Health <= 0`, не-overworld, имена, защита пути, полная очередь, shutdown,
ошибка записи); `net` 213 passed / 4 ignored (+10: entry overworld/
non-overworld/override/нет файла/on-ground, режим force on/off/нет режима/нет
store, `send_join` — телепорт в сохранённую позу и `set_health` 7-м; +3:
снапшот, quit-сейв в файл, автосейв двух игроков); `server` 50, `--test
join`/`--test status` читают `set_health` после телепорта. `cargo test
--workspace` и clippy `-D warnings` зелёные.

**S13 (живой сценарий «вход → движение/поворот → выход → повторный вход»)
не запускался** на момент шага 18: в рабочей среде не было собранного
стенда (`mcproxy`, `server`, Fabric-мод). Статус на тот момент — «не
установлено»; закрыт живым прогоном в шаге 19a.

### 18b — анализ теста (в) `entity_stream_parity` (закрыто как анализ)

- **Что сравнивает тест.** `compare()` идёт по индексам: сначала тип пакета,
  затем payload **побайтно** после `normalized()`. `normalized()`
  нормализует только entity id и поля позиции/дельт; `update_attributes`
  нормализуется **только по `entity_id`** — порядок записей остаётся в
  сравнении. То есть тест **не** сравнивает `update_attributes` как набор
  (исправлено в шаге 19b).
- **Следствие.** Класс «порядок `update_attributes`» (C1 (26,13,8),
  C2 (8,26,13), C4 (26,8,13)) остаётся красным в (в), тогда как
  `docs/DECISIONS.md` объявляет порядок не критерием parity. Тест (в) по
  условию задачи не менялся: расхождение зафиксировано в DECISIONS и здесь,
  вопрос владельцу — менять ли нормализацию (в) на сравнение как набора.
- **C5/C6.** У нас 11 пакетов о наблюдателе против 12 ванильных; лишний —
  `entity_position_sync` о наблюдателе (B о A). Поведение ванили (когда
  именно наблюдатель получает sync о себе) из трейса не установлено —
  минимальную правку сервера не делали, вопрос открыт (разобран в шаге 19b,
  решение за владельцем).
- **Живого прогона не было** (нужен стенд владельца); выводы — из текста
  теста и записанных трейсов.

### Блокеры

- S13 не был выполним на момент шага 18 (нет собранного стенда). Закрыто
  живым прогоном `2026-11-17_s13-live-r7` в шаге 19a (0 failed); см. ниже.

## Шаг 19: живая проверка playerdata (19a), тест (в) (19b), риски (19c)

**19.0 (Шаг 18 / S14).** Проверка «был ли выполнен шаг 18»: да. Мгновенное
ломание и водные блоки закрыты коммитами `b8c7ade`, `55dfd0f`, `147e4b5`,
`41c658c`, `9499d9d`, docs `e782944`; живой прогон
`2026-11-17_s14-ours-r3` — 0 failed.

### 19a — живая проверка S13 (закрыто)

Стенд: release-сервер, `mcproxy`, Fabric-тест-клиент 26.3. Основной прогон —
`E:\MineRust-data\autotests\2026-11-17_s13-live-r7` (0 failed): `save_dir`
внутри каталога прогона, `game_mode = 0`, `force_gamemode = true`,
`autosave_interval_ticks = 200`. Дополнительно:

- `2026-11-17_s13-stop-r1` (0 failed): закрытие консоли сервера (`taskkill`
  без `/F`) → `player saves flushed`, файл новее остановки, dxz 0,000.
- `2026-11-17_s13-gamemode-r2` (0 failed): `force_gamemode=false`, в файле
  `playerGameType=2` (adventure), конфиг `survival` → клиент вошёл в
  adventure, т.е. файл победил.

Проверки r7 (подробности и точные числа — `docs/PLAYERDATA.md` §12):
движение/поворот (yaw 27, pitch 9) сохраняются автосейвом при подключённом A
и на выходе; `Pos`/`Rotation` в файле совпадают с наблюдёнными (dxz 0,000);
схема NBT (`Pos`, `Rotation`, `Health=20`, `foodLevel=20`, `foodSaturationLevel=5`,
`DataVersion=5023`, `playerGameType=0`, `Dimension=minecraft:overworld`,
`OnGround=1`); повторный вход телепортирует в сохранённую позу и поворот
(`player_position`, dxz 0,0000); `force_gamemode=true` берёт режим конфига
(файл 2 → клиент survival); B видит A в сохранённой позе (`add_entity seq=62`,
dxz 0,000); битый файл → спавн + `warn`; шаблон (55 файлов) и
`vanilla\world` (64 файла) не изменились (SHA-256).

Найдено и исправлено по ходу:

- `playerdata::file_name` реверсировал первые три группы UUID-строки вместо
  байтов UUID по порядку: сервер искал `08059210-...`, файл лежит как
  `10920508-...`. Новый тест — на файл ParityBot из шаблона.
- `MoveSlot` держал только последний пакет движения, поэтому
  `move_player_rot`, пришедший перед `move_player_pos` в том же тике, терялся
  и в файл писался `Rotation=(0,0)`. Второй слот (`take_rotation`) +
  `PlayerPos::apply_rotation`; 3 теста в `net`.
- Проверка (5) была вакуумной: `save_all` вызывается после `after_entities`,
  его время не входит в `phases_total_us`, а парсер лог-строк не снимал
  `ESC`-байт ANSI-раскраски. В сервер добавлена строка `autosave queued`
  (`tick`, `save_us`, `players`); в r7 — 17 автосейвов, 2 с двумя игроками,
  `save_us` медиана 19 мкс, максимум 23 мкс (бюджет 5000), очередь не
  переполнялась. `phases_total_us` остаётся контекстом: в него входят
  доставка чанков при входе клиентов и пробуждение ОС, а не запись.

Сценарии харнесса: `tools/testclient/scenarios/s13-playerdata.json`
(`s13`), `s13-stop-check.json` (`s13-stop`), `s13-gamemode-check.json`
(`s13-gamemode`); параметры `-OursSaveDir`, `-OursAutosaveTicks`,
`-OursForceGamemode`. В `scenario-lib.ps1` добавлен
`Get-OfflinePlayerFileName` (имя файла по нику до запуска клиента) и
`Read-Trace` больше не падает на недописанной последней строке прокси-трейса.

**Известная проблема (не шаг 19).** Клиент при загрузке чанков из шаблона
пишет `WARN [LevelChunk] Serialization errors: chunk@[0, 9]
minecraft:trial_spawner ... Registry does not exist: minecraft:trial_spawner`.
Клиент переживает, блок-сущность остаётся пустой: наша configuration-фаза не
отправляет datapack-реестры (в ванили их шлют). Отдельная задача.

**Флейк gate (не шаг 19).** Один прогон `cargo test --workspace` под нагрузкой
(в фоне шёл пересбор графа `graphify`) уронил `bench/tests/smoke.rs` на бюджете
`wall < 5 s`; два повторных прогона — зелёные, тест и его бюджет не менялись.

### 19b — тест (в) `entity_stream_parity`: `update_attributes` как набор (закрыто)

Разрешённая правка теста: в `normalized()` записи `update_attributes`
сортируются канонически — `(key, base bits, modifiers по id/amount bits/
operation)`; это ровно тот набор, который сравнивает
`UpdateAttributes::records_match`. Сравнение остальных пакетов не менялось
(payload побайтно, id/координаты нормализуются, режим игры — нет). Решение —
`docs/DECISIONS.md`.

Остаточные красные (`cargo test -p parity --release --test entity_stream_parity
-- --ignored`; отчёт `E:\MineRust-data\traces\entity-stream-parity.md`,
17.11.2026 — 280 диффов):

| сценарий | режим | пакетов о B (ваниль/мы) | диффов | класс |
|---|---|---|---|---|
| C1 (спавн) | 3 spectator | одинаково | **0** | порядок `update_attributes` больше не расхождение |
| C2 (ходьба) | 3 | 176 / 178 | 169 | лишний/недостающий `entity_position_sync`, чередование `rotate_head` и `move_entity_pos*` (сдвиг с #6), байты yaw в `move_entity_pos_rot` (`0x20` vs `0xa0`) |
| C4 (поворот на месте) | 3 | 113 / 111 | 99 | то же + байты `rotate_head`/`move_entity_rot`, `entity_position_sync` на #10/#27/#30 |
| C5 (спавн) | 0 survival | 12 / 11 | 6 | нет одного `entity_position_sync` перед `update_attributes` (#5), дальше сдвиг |
| C6 (спавн) | 1 creative | 12 / 11 | 6 | то же; байтовое сравнение `update_attributes` (модификаторы креатива) заблокировано сдвигом — **не проверено** |

То есть правка 19b убрала единственный класс, который зависел от порядка
записей (C1 теперь чистый); остальные диффы — поведение ванили, не
нормализация.

### 19c — риски playerdata (закрыто как анализ)

`docs/PLAYERDATA.md` §13: дубликат имени, окно потерь между автосейвами,
смена `save_dir`, offline/online UUID; пять вопросов владельцу (§13.5).
Код не менялся.

### Блокеры / нужны решения владельца

- C5/C6: ваниль шлёт `entity_position_sync` о наблюдателе сразу после
  спавна (`update_attributes` уже после него); мы не шлём. Минимальная
  правка (sync перед `update_attributes` на `ToGame::TeleportAck`) не
  сделана без решения владельца.
- C6: в `update_attributes` ваниль кодирует модификаторы креатива
  resource-location (строки), наш `AttributeModifier` — UUID. Сейчас это не
  проверено: из-за недостающего `entity_position_sync` пакет сравнивается со
  сдвигом, байтовое сравнение `update_attributes` в C6 пропускается. После
  правки C5/C6 формат нужно проверить отдельно.
- `fsync` каталога после `rename` (durability), дефолт автосейва, маркер
  чужого `save_dir` — вопросы §13.5.

## Шаг 20: решения владельца по playerdata (20a), разбор parity (20b), план сохранения мира (20c) (17.11.2026)

Владелец ответил на пять вопросов `docs/PLAYERDATA.md` §13.5: 20a — код
(дубликат имени, автосейв, online, маркер, `fsync`), 20b — только анализ
C2/C4/C5/C6 (тест (в) не менять, сервер не править), 20c — только план
сохранения мира без кода.

### 20a — решения по playerdata (закрыто, код)

1. **Дубликат имени — отключать старое соединение (как ваниль).** Сначала
   снят ванильный S15: `2026-11-17_s15-vanilla-r2` — второй вход с тем же
   ником выбрасывает старый клиент, новый остаётся; в трейсе старого
   `minecraft:disconnect` (play id 32) с NBT-причиной
   `{"translate":"multiplayer.disconnect.duplicate_login"}`, и это последний
   пакет соединения. Код по трейсу: `protocol::DisconnectPlay` (play id
   `0x20`) + побайтный тест на снятых байтах; `KickReason::DuplicateLogin`,
   `ServerMessage::Disconnect`; `ToGame::Join` сравнивает offline UUID и
   старому игроку шлёт `Disconnect`, сохраняет его снимок и удаляет из
   региона; play-цикл пишет пакет и закрывает соединение. Тест `net`:
   `a_second_login_with_the_same_name_kicks_the_first`. Живой прогон на нашем
   сервере `2026-11-17_s15-ours-r1` — **0 failed**: старый клиент выброшен,
   новый остался, ровно один `disconnect` с теми же байтами
   (`0a0800097472616e736c61746500266d756c7469706c617965722e646973636f6e6e6563742e6475706c69636174655f6c6f67696e00`),
   в `save_dir` один файл игрока (267 Б) — снимок выброшенной сессии.
2. **Автосейв 6000 тиков — оставлен.** Код не менялся.
3. **Online-режим — не сейчас.** Зафиксировано в `docs/PLAYERDATA.md` §13.5,
   кода нет.
4. **Маркер `save_dir`.** Пустой каталог при открытии получает
   `minerust-save.json` (`format: "minerust-save"`, `version: 1`,
   `data_version`); непустой без маркера, битый маркер или будущая версия —
   `warn` и работа (не присваиваем). Тесты
   `open_writes_the_marker_into_an_empty_save_root`,
   `open_tolerates_a_non_empty_root_without_a_marker`,
   `open_tolerates_a_damaged_or_future_marker`.
5. **`fsync` каталога.** После `rename` — `sync_dir`: на Unix `fsync`
   каталога, на Windows вызов — no-op (открыть каталог нельзя, переименование
   журналируется NTFS). Тест `a_write_syncs_its_directory_where_supported`
   зелёный на обеих платформах; записано в `docs/PLAYERDATA.md` §13.

`cargo test -p playerdata` — 17 passed. Харнесс: `-Instance` в launcher-
скриптах (два клиента с одним ником в разных каталогах), `Invoke-S15`,
`Get-S15TraceFacts`, `ConvertFrom-TraceDisconnectText`, опрос трейсов до
20 с, `scenarios/s15-duplicate-login.json`; seed-опции в обоих launcher-
скриптах чинят зависание свежего game dir после атласа `shulker_boxes`.

### 20b — разбор parity C2/C4/C5/C6 (закрыто как анализ, кода нет)

Метод: трейсы теста (в) + скрипты разбора в `E:\MineRust-data\traces`
(`c2c4_20b.py`, `c5c6_20b.py`, `sb_20b.py`, `syncs_20b.py`,
`diffcount_20b.py`). Полный текст и таблицы — `docs/MECHANICS.md`, раздел
«Разбор parity C2/C4/C5/C6».

Главное: B в обоих прогонах шлёт одно и то же (C2 — 200 serverbound-
движений, C4 — 100; yaw C2 — чередование `0x20`/`0xa0`, C4 — шаги 9°),
т.е. расхождения не в клиенте, а в том, что сервер отправляет наблюдателю.

| класс | диффов | реально? | причина |
|---|---|---|---|
| нет `entity_position_sync` о наблюдателе | C2 8, C4 4, C5 1, C6 1 пакет | да | мы его не шлём никогда |
| сдвиг выравнивания после лишнего/недостающего пакета | C2 169, C4 89 (тип) + 25 + 8 payload | нет, артефакт | сравнение по номеру пакета |
| `rotate_head` в C4 | 8 payload | да | head yaw ванили отстаёт от body на полшага (≈9°), у нас равен |
| состав/темп движения | C2 103 vs 96, C4 `move_entity_rot` 51 vs 46 | да, причина не установлена | гипотеза: разное окно коалесинга |
| формат модификатора креатива (C6) | 1 пакет | да, скрытая ошибка кодека | modifier = identifier-строка + f64 + u8, наш — UUID |

C5/C6: ваниль шлёт `entity_position_sync` через +291/+170 мс после
`add_entity` (после нулевого `move_entity_pos` и второго `set_entity_data`),
`teleport=1, flags=1, rotation=2`, поза входа, tail `1`; сразу за ним —
`update_attributes`. Что включает периодический sync в C2/C4 (2668…12623 мс,
пары через ~100 мс и паузы до 5,5 с) — **не установлено**. Минимальная
правка (один sync на `ToGame::TeleportAck` перед `update_attributes`) ждёт
решения владельца.

### 20c — план сохранения мира (закрыто как план, кода нет)

`docs/WORLD-SAVE.md`: формат (рекомендация — гибрид C: свой дифф
`chunks/<x>.<z>.mrchunk` поверх шаблона, Anvil-экспорт отдельным этапом),
что/где/когда сохранять (тик только снимает снимок грязных чанков, пишет
отдельный поток), загрузка (наложение диффов до bind), согласованность
(маркер version 2 + `world.json` с SHA-256 `level.dat`, атомарная запись),
связи с playerdata/блок-сущностями/предметами/светом, тесты и живой
сценарий `s16-world-save`, разбивка 21a–21e, риски и 7 вопросов владельцу
(§10). Решений по ним пока нет.

### Блокеры / нужны решения владельца

- C5/C6: минимальная правка `entity_position_sync` на `ToGame::TeleportAck`
  перед `update_attributes` — код не писался, ждёт решения. Она закроет
  C5/C6, но не C2/C4 (триггер периодического sync не установлен).
- C6: формат `AttributeModifier` — resource-location строка вместо UUID;
  правка кодека не делалась (20b — только анализ).
- `docs/WORLD-SAVE.md` §10: формат (B/A/C), совместимость с ванилью,
  проверка идентичности мира, интервал и порог автосейва, объём этапа 1,
  общий ли `save_dir`.
