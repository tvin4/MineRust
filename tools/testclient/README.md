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

```bat
tools\testclient\client.bat Alice 127.0.0.1:25565
tools\testclient\client.bat Bob 127.0.0.1:25565
```

У каждого клиента свой `--gameDir E:\MineRust-data\clients\<ник>` (вне
репозитория). Логи клиента — `<gameDir>\client-stdout.log` /
`client-stderr.log`. Дальность прорисовки и симуляции — 10.

## Что ещё не в репозитории (намеренно)

Скачанные jar/ассеты, папки `clients/`, `autotests/`, токены, собранные jar
мода — не коммитятся (см. `.gitignore`). В репозитории только скрипты,
исходники мода, README и JSON-сценарии.

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

## Статус

- Этап А (батники): готов, приёмка пройдена (оба клиента на ванили 25566
  через mcproxy 25565, `-Xms512M -Xmx2048M`).
- Этап Б (Fabric-мод): B1 (каркас Loom + сборка) готов; B2 (JSON-сокет,
  команда ping) готов, приёмка пройдена; B3 (команды ввода) готов,
  приёмка пройдена трейсом mcproxy: serverbound `player_input` (43) при
  hold движения, `punch` (46) при атаке в воздух, `player_action` (41) +
  `punch` (46) при копании блока; B4 (wait/disconnect/command/screenshot)
  готов, приёмка пройдена; `connect` заблокировано (см. выше); B5 (полная
  приёмка этапа Б) готов: два клиента, у A ввод (43/46/41 в трейсе),
  B наблюдает A через `observe {player}` (поза/углы/режим игры).
- Этап В (оркестратор/сценарии), Г (автотрейсы) — следуют.
