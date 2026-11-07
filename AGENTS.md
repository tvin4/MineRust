# AGENTS.md — правила для ИИ-агента (opencode)

Проект: многопоточный Minecraft: Java Edition 26.3 на Rust (сервер + клиент + лаунчер).
Рабочее название: <PROJECT_NAME>. Полный план: `ARCHITECTURE.md`. Прогресс: `docs/PROGRESS.md`.

## Главные правила
1. Работай ТОЛЬКО над текущим этапом (milestone) из `docs/PROGRESS.md`. Не перескакивай вперёд.
2. Маленькие шаги: один шаг = одна понятная задача = один коммит.
3. Перед каждым коммитом ОБЯЗАТЕЛЬНО: `cargo fmt`, `cargo clippy --workspace -- -D warnings`, `cargo test --workspace`. Если что-то красное — чини, не коммить.
4. Язык кода, комментариев и коммитов: английский (Conventional Commits: `feat:`, `fix:`, `test:`, `docs:`, `refactor:`). Общение с владельцем проекта: русский.
5. После каждого этапа обнови `docs/PROGRESS.md` (что сделано, что дальше, известные проблемы) и сделай `git push`.
6. Не выдумывай детали протокола, ID блоков и числа из памяти. Данные берутся из генератора данных пользовательской копии игры (см. ARCHITECTURE.md, раздел 5). Если данных нет — оставь `TODO` и спроси владельца.

## Юридические границы (строго)
- НЕ копируй код, ассеты и данные Mojang/Microsoft в репозиторий. Ни декомпилированный код, ни текстуры, ни JSON-файлы реестров.
- Код пишется по наблюдаемому поведению и по публичной документации протокола (wiki.vg-подобные описания).
- Лаунчер проверяет владение игрой только через официальные сервисы Microsoft/Minecraft. Никаких обходов проверки и «пиратских» режимов.
- В README держи дисклеймер: «Не является официальным продуктом Minecraft, не одобрен Mojang или Microsoft».

## Стиль кода
- Rust stable, edition 2021 или новее. Workspace из маленьких крейтов.
- Без `unwrap()`/`expect()` вне тестов. Ошибки: `thiserror` в библиотеках, `anyhow` в бинарниках.
- `unsafe` только с комментарием `// SAFETY:` и отдельным тестом.
- Файл не длиннее ~500 строк; функция не длиннее ~60 строк. Дели на модули.
- Публичные типы и функции — с doc-комментариями.
- Многопоточность: никаких `Mutex` в горячем пути тика. Используй передачу сообщений (каналы) и владение данными регионом. Любая новая блокировка требует записи в `docs/DECISIONS.md` с обоснованием.
- Логи через `tracing`, не `println!`.

## Как ориентироваться в проекте (graphify)
- Перед тем как искать по файлам, смотри граф: `graphify query "<вопрос>"` и `graphify-out/GRAPH_REPORT.md`.
- После крупных изменений структуры: `graphify update .` (граф также пересобирается git-хуком при коммите).
- `target/` исключён через `.graphifyignore`.

## Источники истины (порядок)
1. Данные из jar/генератора игры (`data/reports/*`, `reports/packets.json`, реестры из client jar).
2. Сравнение с ванильным сервером через parity-тесты (крейт `parity`).
3. Вики — только как подсказка.

Любая механика без проверки по пунктам 1 или 2 считается непроверенной и в коде
помечается комментарием `// SOURCE: wiki, unverified`. Аудит текущего состояния —
`docs/SOURCES.md`.

## Тесты
- Юнит-тесты рядом с кодом, интеграционные в `tests/`.
- Крейт `parity` сравнивает наш сервер с ванильным (jar берётся у пользователя, в репозитории его нет).
- Любая багфикс-задача начинается с теста, который падает.
- Бенчмарки (`criterion`) для горячих мест: тик региона, генерация чанка, освещение, сериализация пакетов.

## Если застрял
Не угадывай и не ломай соседний код. Запиши проблему в `docs/PROGRESS.md` в раздел «Блокеры», коротко опиши варианты и спроси владельца.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
