# tools/testclient — тестовые клиенты 26.3 для автосценариев видимости (Шаг 9)

Автономная проверка «что видит клиент B у игрока A» (поза, углы, взмах, режим
игры) вместе с сетевым трейсом. Два клиента, ограничение памяти, запуск одной
командой.

**Юридическое.** Здесь нет кода и ассетов Minecraft. Клиентский jar, библиотеки
и ассеты берутся из **уже установленной игры владельца** (ElyPrismLauncher /
`.minecraft`) и подчиняются EULA и лицензии игры владельца. Скачивание ведётся
только с официальных серверов Mojang (piston-meta/piston-data) и Fabric
(meta.fabricmc.net/maven), с проверкой SHA1/SHA256 там, где манифест её даёт.
Реальных учётных данных нет и не используется: токен — заглушка, подключения
только к `127.0.0.1`. Декомпилированные исходники игры не читаются и не
включаются.

## Состав (Этап А)

| Файл | Что делает |
|---|---|
| `client.bat <ник> [адрес]` | запуск одного ванильного клиента 26.3 (обёртка над `launch-client.ps1`) |
| `launch-client.ps1` | правка `options.txt` (дальность 10), старт java, логи в gameDir |
| `New-ClientLaunch.ps1` | сборка команды из `version.json` (main class, classpath, natives, аргументы) |
| `print-launch.ps1` | отладка: печать собранной команды без запуска |

Всё вне Cargo workspace — Rust-код проекта и тесты не трогаются.

## Источники фактов (проверены запуском 14.11.2026)

- **Java**: `java-runtime-epsilon` (Microsoft OpenJDK 25.0.1) из
  `ElyPrismLauncher\java\` — та же, что указана в манифесте версии
  (`javaVersion.component=java-runtime-epsilon`, major 25).
- **Клиентский jar 26.3**:
  `ElyPrismLauncher\libraries\com\mojang\minecraft\26.3\minecraft-26.3-client.jar`
  (sha1 `e877b6a0…` из манифеста; путь подтверждён логом `data::load`).
- **Библиотеки/natives**: `ElyPrismLauncher\libraries` (те же, что качает
  лаунчер), natives распаковываются в temp по правилу из манифеста
  (`${natives_directory}/java`).
- **Ассеты**: `ElyPrismLauncher\assets` (assetIndex id `34`).
- **Аргументы запуска**: секции `jvm`/`game` официального
  `versions/26.3/26.3.json` (из version_manifest_v2). Секция
  `default-user-jvm` лаунчера (ZGC и пр.) намеренно не применяется; лимиты
  памяти по правилу задания: `-Xms512M -Xmx2048M`.
- **Автоподключение**: 26.3 поддерживает `--quickPlayMultiplayer <host:port>`
  (ключ `is_quick_play_multiplayer` в манифесте) — клиент сам заходит на
  сервер. Проверено запуском (см. отчёт Шага 9).
- **UUID offline**: MD5 от `OfflinePlayer:<ник>` с битами RFC 4122 v3 — сверен
  с тем, что показывает ванильный сервер в логе.
- **options.txt**: ключи `renderDistance`, `simulationDistance` сверены с
  настоящим файлом игры 26.3; ставятся в 10.

## Запуск

Ручной клиент (этап А, **без мода и без токенов**; обёртка — `client.bat`):

```bat
tools\testclient\client.bat Alice 127.0.0.1:25565
tools\testclient\client.bat Bob   127.0.0.1:25565
```

Из проводника: двойной клик по `tools\testclient\client.bat` открывает консоль и
просит ник — так удобно для одного клиента; для двух клиентов надёжнее запустить
команду из консоли дважды (или создать два ярлыка с аргументами `Alice` / `Bob`).

Настройки — те же, что у автостенда: отдельный `--gameDir
E:\MineRust-data\clients\<ник>` (вне репозитория), `-Xms512M -Xmx2048M`,
`renderDistance`/`simulationDistance` = 10, offline UUID по правилу
`OfflinePlayer:<ник>` и автоподключение quickPlay на указанный адрес.
Логи клиента — `<gameDir>\client-stdout.log` / `client-stderr.log`.

**Проверено запуском 15.11.2026** (наш сервер, `listen 127.0.0.1:25565`, мир из
`E:\MineRust-data\world`): оба клиента вошли — серверный лог
`player joined player=Alice entity_id=1` и `player joined player=Bob
entity_id=2`; на клиентах дальше видна загрузка чанков.

## Что ещё не в репозитории (намеренно)

Скачанные jar/ассеты, папки `clients/`, `autotests/`, `traces/`, `natives`,
токены и собранный jar мода лежат **вне репозитория** — под `E:\MineRust-data`
(путь задаётся в скриптах: `-ClientsRoot`, `-NativesRoot`, `RunDir`). В
репозиторий они не попадают в принципе; `.gitignore` дополнительно исключает
`target/`, `server.toml`, `graphify-out/` и `__pycache__/`. В репозитории —
только скрипты, исходники мода, README и JSON-сценарии.

## Состав (Этап Б, мод)

Каталог `mod/` — Fabric-мод, который превращает клиент 26.3 в управляемый
тестовый клиент (JSON-команды по локальному сокету). Исходники мода в репо;
jar-сборка, кэш Gradle/Loom и run-каталоги — нет.

| Файл | Что делает |
|---|---|
| `mod/build.gradle`, `settings.gradle`, `gradle.properties` | каркас Loom по официальным версиям Fabric (см. ниже) |
| `mod/src/main/resources/fabric.mod.json` | манифест мода (`id=testclient`, entrypoint `client`) |
| `mod/src/main/java/dev/minerust/testclient/` | код мода: `TestClientMod` (entrypoint), `ControlServer` (JSON-сокет 127.0.0.1), `CommandDispatcher` (команды на клиентском тике), `InputCommands` (`look`/`key`), `ObserveCommand` (`observe`), `SessionCommands` (сессия), `TickQueue` (планировщик тиков) |
| `Invoke-Rcon.ps1` | RCON-клиент консоли ванили (`save-all flush`, `/tp`, `/gamemode` для оркестратора; RCON включён в `server.properties` стенда, только 127.0.0.1) |

**Версии (все — официальные источники Fabric/Mojang, проверены 14.11.2026):**
Minecraft `26.3`, fabric-loader `0.20.1` (meta.fabricmc.net/v2/versions/*),
fabric-loom `1.19.4` (maven.fabricmc.net), Fabric API `0.166.2+26.3`
(maven.fabricmc.net), Gradle `9.9.1` (services.gradle.org, SHA256 сверен),
Java 25 (JDK Adoptium 25.0.4 — совместим с runtime клиента 25.0.1).

**Сборка** (из каталога `mod/`, Gradle-дистрибутив и кэш — на E:):

```powershell
$env:JAVA_HOME = 'C:\Program Files\Eclipse Adoptium\jdk-25.0.4.7-hotspot'
$env:GRADLE_USER_HOME = 'E:\MineRust-data\gradle-home'
& 'E:\MineRust-data\gradle-dist\gradle-9.9.1\bin\gradle.bat' build
```

Артефакт: `mod/build/libs/testclient-mod-<версия>.jar`. Loom скачивает
Minecraft и библиотеки только с официальных серверов Mojang/Fabric
(piston, launcher.mojang.com, maven.fabricmc.net).

**Запуск Fabric-клиента** (мод + сокет, команды — newline-delimited JSON):

```powershell
& tools\testclient\launch-fabric-client.ps1 -Nick Alice -Address 127.0.0.1:25565 -ControlPort 25571
```

Протокол сокета: запрос `{"id":1,"cmd":"ping","args":{}}` (по строке),
ответ `{"id":1,"ok":true,"result":{...}}` либо `{"id":1,"ok":false,"error":"..."}`.
Сокет слушает только `127.0.0.1`, по умолчанию порт `25571`
(переопределяется `-Dtestclient.port`). Мод ставится в `<gameDir>/mods`,
Fabric loader + fabric-api берутся с официального maven.fabricmc.net
(SHA1 сверяется) и кэшируются в `E:\MineRust-data\testclient-libs`.
Профиль клиента — официальный JSON Fabric
(`meta.fabricmc.net/v2/versions/loader/26.3/0.20.1/profile/json`).

**Команды этапа Б3** (все только через клиентский путь ввода, ни одного
пакета вручную; имена пакетов/идентификаторы —
`E:\MineRust-data\reports\packets.json`):

- `look {dx, dy}` — дельты мыши через `Entity.turn(double,double)`.
  Измерено: 1000 дельт по вертикали ≈ 150° (0.15°/дельта при сенсе по
  умолчанию); точные углы после поворота читаются через `observe`.
- `key {key, action, hold_ms}` — `action` = `down`/`up`/`tap` (по умолчанию
  tap), `hold_ms` длительность tap (по умолчанию 100). Ключи:
  `up/down/left/right/jump/shift/sprint` — held-клавиши через
  `KeyMapping.setDown` (путь GLFW-состояния; сервер получает
  `player_input`, id 43, при изменении input — подтверждено hold ≥ 2 с);
  `attack`/`use` — клавиши-биндинги через `KeyMapping.setDown` +
  `KeyMapping.click(getDefaultKey())` (путь GLFW-клика; сервер получает
  `punch`, id 46, и `player_action`, id 41, при копании — подтверждено);
  `swing` — `player.swing(MAIN_HAND, SwingAnimation.DEFAULT, broadcast)`
  (сам по себе 46 не даёт; используется как часть сценария).
  Путь `path:"mouse"` (`MouseHandler.onButton`) — **только диагностический,
  сценариями не используется** (решение владельца, Б4): он не производит ни
  41, ни 46. Для S3: взмах в воздух и удержание на блоке различаются по
  трейсу (удержание даёт много 46 — факт и порядок пакетов, количество не
  считается).
- `observe` — снимок состояния: позиция, yaw/pitch, режим игры, версия,
  maxMemory, `windowActive`, `mouseGrabbed`, `paused`, `keyAttackDown`,
  `keyUseDown`, тип цели прицела (`MISS`/`BLOCK`/`ENTITY`) и блок под
  прицелом. `screen == null` в 26.3 публичного API не имеет — факт
  «экран открыт» заменён эквивалентом `mouseGrabbed == false`.
  Форма `observe {player:"Alice"}` — что видит этот клиент у другого
  игрока (поза/углы из `ClientLevel.players()`, режим игры из
  player-info): это и есть проверка «клиент B у игрока A» (Б5, подтверждено
  запуском: yaw A у B 12.66 ≈ локальные 12.75 с учётом квантизации углов
  сервером, режим игры совпадает).

**Команды этапа Б4** (сессия):

- `wait {ms}` — ответ приходит после `ms` реальных клиентских тиков
  (мир продолжает тикать; клиентский тред никогда не блокируется).
  Таймаут сокета поднимается автоматически (мс + 3 с).
- `disconnect` — `Minecraft.disconnectFromWorld(Component)` (путь кнопки
  Disconnect): сервер сразу видит чистый `Disconnected`. Альтернатива
  `disconnectWithSavingScreen` (попытка 1) обнуляет игрока, но оставляет
  соединение до keepalive-таймаута — не используется.
- `command {text}` — чат-команда через `ClientPacketListener.sendCommand`
  (путь экрана чата); ответ сервера приходит как chat, не в ответе.
- `screenshot {name?}` — `Screenshot.takeScreenshot` (путь F2) пишет
  `<gameDir>/screenshots/<name>.png` и возвращает путь + факт файла.
- `connect` — **заблокировано, причина не установлена** (две попытки,
  третья не делается — решение владельца, Б4). Гипотезы (не проверены,
  декомпиляция запрещена): (а) parent-экран обязателен ненулевой
  (`JoinMultiplayerScreen`); (б) приватное `Minecraft.pendingConnection`
  блокирует повторный connect; (в) `ServerData` требует
  инициализированного состояния. Команда остаётся в коде; сценарии
  используют запуск клиента сразу подключённым (quickPlay
  `launch-fabric-client.ps1`).

**Команды сбора данных из jar** (данные игры, не сценарии):

- `shapes {path}` — пишет `{path}` = `reports/collision.json`: для каждого
  непустого состояния `BlockState.getCollisionShape(level,pos).min(Y)`
  (низ коллизионных коробок в клетке), измеренный в воздушной клетке над
  игроком. Источник правила ползания (Шаг 15 п.5, `collision_report`).
- `hardness {path}` — пишет `{path}` = `reports/hardness.json`: для каждого
  состояния `BlockState.getDestroySpeed(level,pos)` (твёрдость; `< 0` —
  неразрушимо) и `Player.getDestroySpeed(state)` (скорость копания держащим
  руку пустой), плюс список неразрушимых id. Источник серверного таймера
  копания (Шаг 16a, `hardness_report`). Обе команды читают только публичный
  API игры и пишут файл вне репозитория.

**Диагностика входа атаки (Б3, решение владельца: сначала факты).**
Факты `observe` до/во время/после атаки: при пустом приёме
`MouseHandler.onButton` (все ворота открыты — окно активно, мышь
захвачена, экранов нет, прицел на блоке) состояние `keyAttackDown`
остаётся `false` и серверных пакетов нет. Путь через `KeyMapping`
(`setDown` + `click`) — `keyAttackDown: true` во время удержания,
копание начинается, трейс: `player_action` ×4 + `punch` ×19; атака в
воздух (тип прицела `MISS`): ровно 3 `punch` на 3 тапа. Принятый путь
атаки — KeyMapping (вариант 2); вывод «реализуемость через
`MouseHandler.onButton`» — SOURCE: наблюдение запуском, механизм отбора
внутри клиента не изучался (декомпиляция запрещена).

## Этап В: автосценарии видимости A→B

`run-scenario.ps1` — оркестратор: стенд (ваниль + mcproxy), клиенты Alice/Bob,
сценарии из `scenarios/s1..s4.json`, проверки по `observe`-командам мода и трейсам
mcproxy, итог `summary.json` в каталоге `E:\MineRust-data\autotests\<дата-время>\`.
Стенд поднимается и гасится автоматически; клиенты убиваются между сценариями
(живой процесс держит natives и после `disconnect` валидно сидит на экране списка
серверов — это не баг входа).

- **S1** (поза через перезапуск, сценарий владельца): A заходит, поворачивается
  (tap движения — см. находки), отключается; `save-all flush` по RCON; процесс A
  убит и перезапущен через quickPlay; затем B. Проверки: (1) B видит ненулевые
  yaw/pitch A в течение 500 мс после входа; (2) поворот пережил перезапуск — по
  NBT playerdata (primary) ИЛИ по трейсу входа A (secondary).
- **S2** (углы): три поворота, B наблюдает дельты yaw/pitch по seq (tolerance 1.5°).
- **S3** (взмах): воздух — punch без player_action; удержание на блоке —
  player_action + punch после; порядок по seq. Количество не считается.
- **S4** (game mode): B видит survival → creative (RCON `gamemode`) → survival.

### Находки отладки (26.3, по трейсам mcproxy)

1. **Поворот не уходит серверу сам по себе**: `look` меняет углы только на клиенте;
   serverbound `move_player_rot` / `player_input` появляются после тапа движения
   (`up` tap) — он проталкивает углы. Сценарии делают tap после каждого поворота.
2. **Клиент готов к командам не сразу**: `player_loaded` уходит ~3.5 с после входа;
   команды, посланные сразу после join-строки лога, молча не отправляются по проводу.
   `Start-Client` ждёт join-строку + 5 с.
3. **Протокол big-endian**: float в пакетах и в NBT читаются BE (BitConverter LE даёт
   мусор). `Find-RotationInTrace` и `Get-PlayerRotationFromNbt` читают BE.
4. **playerdata 26.3**: `<world>/players/data/<uuid>.dat`, первые три группы UUID
   в имени файла реверсятся по байтам (08059210-... → 10920508-...). Тег `Rotation`
   — TAG_List из двух float BE.
5. **`Write-Host` не попадает в pipeline**: launch-скрипт дублирует offline-uuid
   через `Write-Output`, иначе оркестратор не находит playerdata.

## Этап Г: прогоны против нашего сервера, таблица «байты → эффект»

`run-scenario.ps1 -Target ours` — тот же оркестратор и те же сценарии, но вместо
ванили за mcproxy (127.0.0.1:25565 → 25566) стоит наш `server.exe` (конфиг
собирается из рабочего `server.toml` с заменённым `listen`; наш сервер слушает
25566 напрямую — порт upstream mcproxy берётся из ванильного `server.properties`).
У нашего сервера нет RCON и playerdata, поэтому `save-all flush` и `gamemode`
в этом режиме пропускаются с явной записью пробела в отчёт, а join-gate идёт по
self-`observe` (`inGame`), а не по логу ванили.

Прогон **2026-11-14_1735** (чистый): S2 **6/6 PASS**, остальное — честные пробелы
возможностей нашего сервера (не баги стенда). S4 **3/3 PASS** остался за ванилью
(там RCON есть).

### Таблица «байты → эффект» (26.3, наш сервер, трейсы 2026-11-14_1735)

Что клиент шлёт (serverbound, id из `reports/packets.json`) → что видит второй
клиент B (факты `observe` + clientbound-трейс B):

| Байты (A → сервер) | Что делает наш сервер | Эффект у B |
|---|---|---|
| 32 `move_player_rot` (S2: 3, S3: 2) | `store_move` → тик `apply_move` | **углы доходят**: 3 clientbound `move_entity_rot` (57) у B, дельты yaw/pitch сходятся (6/6, ошибка ≤ 0.7° — квантование 1.40625°) |
| 31 `move_player_pos_rot` (0) / 30 `move_player_pos` | `store_move` → `apply_move` | поза доходит: `entity_position_sync` (35) при прыжках за пределы delta-кодирования; B видит координаты A |
| 43 `player_input` (S2: 6, S3: 0) | `store_input` → `apply_input` | флаги в состоянии игрока; с шага 10 подтверждённые переходы sneak/sprint дают B `set_entity_data` (101) `02 → 00 → 08 → 00` + `update_attributes` (134) для sprint (S5, `01 00 00 02 ff` …) |
| 46 `punch` (S3: 4) | `store_swing` → тик `record_swings` | **взмах невидим**: clientbound `swing_animation` (123) / `animate` у B — 0 (броадкаст не подключён: `drain_swings` не вызывается) |
| 1 `attack` (S3: 4) | игнорируется (default-ветка) | нет эффекта |
| 41 `player_action` (S3: **0**) | — | **копание не стартует**: на ванили удержание на блоке даёт `player_action` ×4 + `punch` ×19, у нас — только `attack`(1) + `punch`(46); клиент не считает это копанием (SOURCE: наблюдение, причина не установлена — чанки присланы, 473 `level_chunk_with_light`) |
| 44 `player_loaded` | маркер готовности клиента | join-gate для `-Target ours` (self-`observe` `inGame`) |

Что B получает от сервера (clientbound) → что проверяет `observe` у B:

| Байты (сервер → B) | Кто шлёт | Факт у B (`observe {A}`) |
|---|---|---|
| 57 `move_entity_rot` (3) | повороты A | `rot.yaw/pitch` — **дельты сходятся** (S2 6/6) |
| 35 `entity_position_sync` / 33 `move_entity_pos` | движение A | `pos` — координаты A |
| 1 `add_entity` (1) | вход A | `found: true`, A появляется в мире B |
| 73 `player_position` (1 при входе) / 74 `player_rotation` (0) | телепорт входа | вход в спавн; поворот клиента при входе — 74 не шлётся |
| 123 `swing_animation` (0) | **не шлётся** | взмахи A невидимы (S3 check 3 FAIL — ожидаемый пробел) |
| `player_info_update` (game mode) | вход A | `gameMode: survival` (S4 check 1 PASS); смена режима — только консолью, у нашего сервера нет RCON (S4 check 2/3 — записанный пробел) |

### Пробелы возможностей нашего сервера (результат этапа Г, не баги стенда)

1. **Сохранение позы при перезаходе** (S1): playerdata не пишется, `player_rotation`
   (74) при входе не шлётся — B видит yaw=0/pitch=0 (ваниль: сохранённые 60°/37.95°).
   Решение о персистентности — за планом M-этапов (реестр: `docs/MECHANICS.md`, п. 11).
2. **Броадкаст взмахов** (S3 check 3) — **закрыт** (коммит `c2011c2`,
   `feat: broadcast swing_animation to observers on accepted punches`). Live
   S3-air `2026-11-15_1857` на нашем сервере: 3 тапа → 3 clientbound
   `swing_animation` у B, payload `VarInt id + 00 01 06` (чек `every swing
   payload is VarInt id + 00 01 06` PASS). Ванильная связь punch→swing
   по-прежнему не один-к-одному (шаг 10 C, `docs/PROGRESS.md`).
3. **`player_action` (копание)** (S3 check 2) — **открыт**: клиент не присылает
   41; у нашего сервера нет ни декодера 41, ни пути изменения блока, поэтому
   ломание блока не работает (ручная проверка владельца 15.11.2026: блок
   исчезает только у копающего клиента — клиентский прогноз; наблюдатель блок
   не теряет). План — `docs/BLOCK-BREAKING.md`.
4. **Смена game mode** (S4 checks 2/3): у сервера нет RCON/командной строки.

Эти пробелы — содержимое плана следующих M-этапов; тест-харнес их фиксирует,
чтобы закрытие пробелов сразу подтверждалось PASS-ом тех же сценариев.

## Статус

- Этап А (батники): готов, приёмка пройдена (оба клиента на ванили 25566
  через mcproxy 25565, `-Xms512M -Xmx2048M`). Перепроверено 15.11.2026 на
  нашем сервере (`127.0.0.1:25565`): оба клиента вошли (шаг 12, п. 6).
- Этап Б (Fabric-мод): B1 (каркас Loom + сборка) готов; B2 (JSON-сокет,
  команда ping) готов, приёмка пройдена; B3 (команды ввода) готов,
  приёмка пройдена трейсом mcproxy: serverbound `player_input` (43) при
  hold движения, `punch` (46) при атаке в воздух, `player_action` (41) +
  `punch` (46) при копании блока; B4 (wait/disconnect/command/screenshot)
  готов, приёмка пройдена; `connect` заблокировано (см. выше); B5 (полная
  приёмка этапа Б) готов: два клиента, у A ввод (43/46/41 в трейсе),
  B наблюдает A через `observe {player}` (поза/углы/режим игры).
- Этап В (оркестратор S1–S4): готов, приёмка пройдена — **14/14 PASS** на ванили
  (чистый прогон 2026-11-14_1644, без ручного ввода в окнах клиентов; загрязнённые
  ручным вводом прогоны не учитывались).
- Этап Г (прогоны против нашего сервера, таблица «байты → эффект»): готов —
  прогон 2026-11-14_1735 (S2 6/6 PASS; S1/S3/S4 — зафиксированные пробелы
  возможностей нашего сервера, см. выше), таблица и выводы в разделе выше.
  Попутно найден и починен баг entry-point нашего сервера (двойная инициализация
  tracing паниковала до разбора аргументов; guard-тест `crates/server/tests/entrypoint.rs`).
