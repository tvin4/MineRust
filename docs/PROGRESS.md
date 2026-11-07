# PROGRESS.md — прогресс проекта

Полный план: `ARCHITECTURE.md` (раздел 8 — этапы M0–M10).
Правила для агента: `AGENTS.md`.

**Текущий этап: M3 — крейт `parity` готов, срезы status и join прогнаны, таблица расхождений показана владельцу. После живого прогона M2 починен состав registry_data/update_tags по данным игры: `registries_report` в конфиге, все 15 теговых реестров (было 7); после второй живой ошибки («Missing element block_transformer / shovel») в registry_data добавлены 3 недостающих реестра — теперь все **32 реестра / 432 записи, состав и порядок записей = ванили** (parity-тест `registry_data_parity`). **Часть 2 (правки по таблице расхождений среза 1) — п.2.1–2.5 выполнены и закоммичены** (нормализация + motd, status_response байт-в-байт, login_finished session uuid v4, login play-поля + hashed_seed, player_info_update маска 0xFF, keep-alive 15 с с киком, `DECISIONS.md` про Known Packs/сжатие, `SOURCES.md`). Следующий шаг: п.2.6 — перегенерировать отчёты `target/parity-report/*-diff.md`, показать обновлённую таблицу расхождений и ждать живой проверки владельца (`cargo run -p server`); после «часть 2 принята» — часть 3 (чанки).**

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
- Открытые точки шага 3.2: block entities в payload (`0a` + 10×5 байт,
  типы VarInt вместо ASCII) — внутренний формат не расшифрован; второе
  u16-поле заголовка = числу клеток воды (одна наблюдаемая точка,
  семантика water/fluid — `// SOURCE: wiki, unverified`).

## Что дальше

- **Часть 3, шаг 3.3 (ждёт подтверждения владельца после показа 3.2):**
  кодеки `level_chunk_with_light`/batch/center в `protocol` + отправка
  чанков в play, критерий — байт-в-байт совпадение payload.
- Живая проверка: владелец запускает `cargo run -p server` (в логе `game
  data loaded (registries=32, ..., tag_registries=15)`), проверяет список
  серверов (теперь motd «A Minecraft Server», как у ванили) и вход клиентом
  26.3. Если клиент падает — причина из его `disconnect-*.txt` +
  пакет-за-пакетом сравнение с ванилью (`registry_data_parity`/`trace_inspect`).
- M3: чтение ванильного мира, отправка чанков — условие успеха: игрок видит
  мир из ванильного сохранения. Каркас `parity` готов (срезы status/join
  прогнаны, дифф-движок и отчёты работают).
- M4+: сжатие (Set Compression), когда Registry Data/чанки перестанут
  влезать в лимит фрейма (решение — `docs/DECISIONS.md`).

## Известные проблемы / блокеры

- M2 закрывает play-состояние только join-последовательностью: чанков нет,
  движение игрока принимается, но не применяется (пустой мир, симуляция —
  M3+). Кик по неответу keep-alive реализован (30 с после неотвеченного
  keep-alive, `ConnectionError::KeepAliveTimeout`; само значение — вики,
  unverified).
- `hashed_seed` Login (play) = 0, пока у нас нет мира: формула уже сверена
  с трейсом ванили (`protocol::hashed_seed`), значение придёт с
  level.dat в части 3. (В шаблоне `level.dat`: spawn `[96,145,−32]`,
  `world_gen_settings.dat`: seed 12345 → base-64 из трейса сверять оттуда.)
- Блокер палитры чанков снят: `C:\MineRust-data\reports\blocks.json`
  сгенерирован владельцем (`tools\generate-reports.ps1`) и прописан в
  `server.toml`; mapping имя+свойства → state id работает (сверено на
  всех 14 секциях шага 3.2).
- `parity` прогон `join_slice` оставляет живым `target\debug\server.exe`
  (пришлось убивать вручную перед `cargo test --workspace` — файл не
  удалялся при ребилде): в скрипте джоина сервер останавливается не
  всегда (флейк, разобрать отдельно).
- Join-состав всё ещё расходится (вензель, чанки, 3748 vs 46 пакетов и
  `select_known_packs`) — это ожидание части 3/решений из
  `docs/DECISIONS.md`, не забытые правки.
- Ид keep-alive у ванили = аптайм машины в мс, у нас — мс от старта
  сервера: origin разный по построению, побайтовое совпадение id не
  достигается и не преследуется.
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
- Живым клиентом 26.3 пока не подтверждено ничего (ручная проверка M2 за
  владельцем) — это самый крупный открытый пункт по источникам.
- `protocol_version`/`version_name` и путь к клиентскому jar из игры
  хранятся только в локальном gitignored `server.toml` владельца; в
  репозитории лежит `server.example.toml` с плейсхолдерами (0 / "unknown").
