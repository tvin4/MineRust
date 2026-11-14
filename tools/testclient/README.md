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
| `mod/src/main/java/dev/minerust/testclient/` | код мода (B1 — скелет, B2+ — сокет и команды) |

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

## Статус

- Этап А (батники): готов, приёмка пройдена (оба клиента на ванили 25566
  через mcproxy 25565, `-Xms512M -Xmx2048M`).
- Этап Б (Fabric-мод): B1 (каркас Loom + сборка) готов; B2–B5 (сокет,
  команды ввода через клиентский путь → serverbound id 43/46, приёмка) —
  в работе.
- Этап В (оркестратор/сценарии), Г (автотрейсы) — следуют.
