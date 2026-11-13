# PROGRESS.md — прогресс проекта

Полный план: `ARCHITECTURE.md` (раздел 8 — этапы M0–M10).
Правила для агента: `AGENTS.md`.

**Текущий этап: M3, шаг 3.3 — 6-пунктовка владельца выполнена (отчёт live-vs-vanilla, матрица chunk-diff, причина block entities (7,−2) найдена и записана в `docs/DECISIONS.md`, guard флейка, SOURCES/PROGRESS). Живая проверка 08.11.2026: M1/M2/3.3 подтверждены настоящим клиентом 26.3. Следующий шаг — план M4 (тик-цикл, позиция игрока, стриминг чанков, регионы/планировщик) показан владельцу, ждём «план принят»; до него никакого кода M4. Движок света не трогаем. Шаг 3.3 (коммиты `504de49..192a806` + этот docs):** a) кодеки `level_chunk_with_light`/`chunk_batch`/`set_chunk_cache_center` с round-trip к записанным байтам; b) правило готовности чанка с именованной причиной отказа («жди генерации M6»); c1) конвертация сохранённого чанка в payload (heightmaps под id трейса, палитры через `blocks.json`, свет §141); c2) актор-загрузчик — пул 2 `std::thread`, `net` зависит от мира только через трейт `net::loader::ChunkSource`, без Mutex в тике; e) play-поля из сохранения — спавн `[96,145,−32]` из `level.dat` (байты позиции сошлись с трейсом ванили), `hashed_seed` из `world_gen_settings.dat` (seed 12345), центр кэша и позиция спавна; d) пейсинг подбатчей по `chunk_batch_received` (KEY из отчёта, ID 0x0B, 50 мс на подбатч); f) парити-срез `chunk_slice` → `target/parity-report/chunk-diff.md` (9/9 чанков сопоставлено, свет 0/9 — таблица показана владельцу, ничего не «чинится»); g) `SOURCES`/`DECISIONS`/`PROGRESS`. Между ними: рефакторинг `connection.rs` на модули (лимит длины файла) и стабилизация keep-alive-теста. Ранее в M3/части 2: parity-каркас, срезы status/join прогнаны, **32 реестра / 432 записи registry_data = ванили**, часть 2 п.2.1–2.5 принята владельцем. Часть A2 (лог server: порядок чанков, кэш бата, уровни лога, дистанции из `server.toml`, пейсинг порций) закрыта — коммиты `d271317`, `da8d2ee`, `e8376a5`, `4917d09`, `6d113a8`, см. раздел «Часть A2». **Следующий шаг: реакция владельца на план M4 («план принят»).**

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
      `C:\MineRust-data\server.jar` (вне репозитория).
- [x] `C:\MineRust-data\vanilla`: первый запуск создал `eula.txt` и
      `server.properties`; EULA принят владельцем → `eula=true` (единственная
      правка файла). `server.properties`: `level-seed=12345`,
      `online-mode=false`, `server-port=25566`, `view-distance=10` (дефолт),
      `level-type=minecraft:normal`, дополнительно `white-list=false`
      (дефолт 26.3 — true, блокировал бы offline-вход тест-клиента).
- [x] Мир сгенерирован с сидом 12345 — проверено разбором gzip+NBT: тег
      `seed` (long) в `world/data/minecraft/world_gen_settings.dat` = 12345
      (в `level.dat` 26.3 сида больше нет). Мир сохранён чисто (`stop`,
      exit=0) и скопирован в шаблон `C:\MineRust-data\world-template`.
- [x] `server.toml` (gitignored): добавлены `vanilla_server_jar` и
      `vanilla_server_dir` (оба пути — вне Git). `ServerConfig` получил
      опциональные `vanilla_server_jar` /
      `vanilla_server_dir` (валидация непустых, тесты парсинга);
      `server.example.toml` — закомментированные примеры.
      `git check-ignore` подтверждён: ни `server.toml`, ни `C:\MineRust-data`
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
- **Правки плана учтены:** `blocks.json` отсутствует в `C:\MineRust-data\
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
   (`C:\MineRust-data\world-template-v1.sha256.txt`, агрегат
   `2f6c0cd4…b00d41`), копия `C:\MineRust-data\world-template-v1`,
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
   (31 443), снапшоты `C:\MineRust-data\world-v2-run{A,B}`, эталон =
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
  min/median/max: `C:\MineRust-data\traces\preload-report.md` + 14 JSONL.
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
  `C:\MineRust-data\traces\part1-move-bytes.md`.
- II.1 (`7bf19aa`): ожидание `select_known_packs` условное - `PROBE_WAIT` 3 с
  только для ванильного порта, `PROBE_WAIT_NO_PROBE` 100 мс иначе; ванильные
  серии не тронуты.
- II.2: только наши серии пересняты (`preload2-ours-*`), отчёт
  `C:\MineRust-data\traces\preload-report-ours-2.md`. Cold: int->441-й чанк
  8609.6/8613.3/8718.2 мс, warm 8585.0/8610.7/9477.8 (было 12047/13034/13434
  и 13865/13916/14410 со старым 3-с ожиданием).
- II.3: разбор payload (`chunk_breakdown.py`, зеркало `chunk_payload.rs`,
  самопроверка по длине). Гипотеза владельца подтверждена: разница join
  17.97 MB на 441 чанк = 19.90 лишних массивов света по 2048 B; наш сервер
  шлёт 24 sky-массива в каждом чанке, ваниль - только непустые (2..7).
  Heightmaps и section blob совпадают. Разбор:
  `C:\MineRust-data\traces\ii3-payload-breakdown.md`.
- III (float `chunks_per_tick`): ваниль warm и наш warm, pace
  {4.6715, 15, 50} x3, все 18 — `Completed` по 441. Число порций:
  ваниль 108/29/12, наш 87/29/9. Время до 441-го (медиана): ваниль
  5921.6/5671.1/5601.3 мс, наш 8545.5/8537.3/8660.1 мс — float задаёт
  размер порций, но время входа меняет слабо. Отчёт
  `C:\MineRust-data\traces\preload-report-float.md`.

## Часть M4.0 (а), раунд 4: ЧАСТЬ I — kick-эксперименты и правило тик/ход (выполнено)

Инструмент `crates/parity`, код сервера не менялся. Полные данные —
`C:\MineRust-data\traces\part1-report.md` (в репозиторий не кладётся).

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
`C:\MineRust-data\traces\part2-report.md`; прогон `part2-run.log`.

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
менялся. Отчёт — `C:\MineRust-data\traces\part3-report.md`, прогон
`part3-run.log`, мир — `C:\MineRust-data\world-template-v3` (в репе только
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
  `C:\MineRust-data\`, затем `pause`; иначе `cargo run -p server
  --release`) и `tools\run-server-vanilla.bat` (только `java -jar
  server.jar nogui` с рабочим каталогом `C:\MineRust-data\vanilla`,
  jar ищется в `vanilla\` затем `C:\MineRust-data\server.jar`; мира не
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
  простоя, release). `C:\MineRust-data\traces\m43-jitter.md`.** Э1
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
  - реальный файл `C:\MineRust-data\world-template\data\minecraft\world_gen_settings.dat`:
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
- Блокер палитры чанков снят: `C:\MineRust-data\reports\blocks.json`
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
  C:\MineRust-data\world-template 4,0`, статус `minecraft:full`), секции
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
  копии `C:\MineRust-data\world-template`. Фон: CPU ~14 %, ~21 ГБ
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
  сырые отчёты `C:\MineRust-data\traces\`, разборы `target/bench/`.
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
  `C:\MineRust-data\traces\entity-stream-parity.md`. Типов: С1 11/8, С2 176/316,
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
  порядок `update_attributes` — не правило; триггер `entity_position_sync` —
  «не установлено»; инструменты `tools/trace-analyze/step6*.py`.
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

## Блокеры

- **Решения владельца по остаткам (в) Шага 6.3** (п. 1–4 выше): нормализовать
  ли порядок `update_attributes`; принять ли эвристику `entity_position_sync`;
  какое правило `rotate_head` (C2 и C4 противоречат); мириться ли с джиттером
  сэмплинга. До решения точное побайтовое совпадение C2/C4 недостижимо.
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
