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
