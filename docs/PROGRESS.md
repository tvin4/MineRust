# PROGRESS.md — прогресс проекта

Полный план: `ARCHITECTURE.md` (раздел 8 — этапы M0–M10).
Правила для агента: `AGENTS.md`.

**Текущий этап: M3 — крейт `parity` готов, срезы status и join прогнаны на ванили 26.3 и на нашем сервере. Следующий шаг: дифф трейсов → таблица расхождений владельцу (стоп до подтверждения).**

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
      1, flags 0), затем keep-alive каждые 10 с (cb 0x2D / sb 0x1C).
- [x] Пакеты play-состояния в `protocol` + тип Position (x26/z26/y12),
      Int/Float/Double в курсоре.
- [x] Таймаут 30 s — только до play (handshake, status, login,
      configuration); в play — кик по неответу keep-alive пока `TODO`.
      Чтение play-фреймов идёт в фоновой задаче через mpsc-канал, чтобы
      ветка keep-alive не отменяла наполовину прочитанный фрейм.
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

## Что дальше

- **Срез 1, стоп по согласованию:** таблица расхождений
  status+login+configuration+join показана владельцу отдельно
  (отчёты `target/parity-report/*.md`). **Ничего в сервере не чинить до
  подтверждения** — починка может начаться только по этой таблице.
- Ручная проверка M2 владельцем: `cargo run -p server` с `client_jar` в
  `server.toml`, вход клиентом 26.3 — мир должен открыться без вылета
  (пока пустой: чанки нет, `TODO` до M3). Рекомендуется сразу сгенерировать
  отчёт (`tools\generate-reports.ps1`) и добавить `packets_report` в
  `server.toml` — тогда ID берутся из игры, а не с вики.
- M3: чтение ванильного мира, отправка чанков — условие успеха: игрок видит
  мир из ванильного сохранения. **Первый шаг M3** — каркас крейта `parity`
  (см. `docs/SOURCES.md`, раздел «План: крейт parity»).
- M4+: сжатие (Set Compression), когда Registry Data/чанки перестанут
  влезать в лимит фрейма; кик по неответу keep-alive.

## Известные проблемы / блокеры

- M2 закрывает play-состояние только join-последовательностью: чанков нет,
  движение игрока принимается, но не применяется (пустой мир, симуляция —
  M3+). Кик по неответу keep-alive не реализован (`TODO` в `net`).
- `hashed_seed` Login (play) = 0 до появления worldgen (`TODO`).
- Детали протокола 26.3 не выдумываются: всё берётся из генератора данных
  и публичной документации протокола; недостающее помечать `TODO`.
  Теперь правило усилено: вики — только подсказка, непроверенное помечается
  `// SOURCE: wiki, unverified`, см. `AGENTS.md` и `docs/SOURCES.md`.
- Без `packets_report` в конфиге ID пакетов — дефолты из вики (неверифицированные
  количественно правы для 26.3, но не закреплены на CI): сервер при старте
  пишет warn; парити-тесты (план в `docs/SOURCES.md`) закроют пункт 2.
- Живым клиентом 26.3 пока не подтверждено ничего (ручная проверка M2 за
  владельцем) — это самый крупный открытый пункт по источникам.
- `protocol_version`/`version_name` и путь к клиентскому jar из игры
  хранятся только в локальном gitignored `server.toml` владельца; в
  репозитории лежит `server.example.toml` с плейсхолдерами (0 / "unknown").
