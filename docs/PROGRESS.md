# PROGRESS.md — прогресс проекта

Полный план: `ARCHITECTURE.md` (раздел 8 — этапы M0–M10).
Правила для агента: `AGENTS.md`.

**Текущий этап: M0 — Каркас (выполнен). Следующий: M1 — Протокол.**

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

## Что дальше

- M1: протокол — handshake, status (крейт `crates/protocol`).
  Условие успеха: сервер виден в списке серверов клиента 26.3.
- Собрать граф проекта: `graphify update .` (после появления первого кода).

## Известные проблемы / блокеры

- Реестры (блоки, предметы, ID) ещё не выгружены из пользовательской копии
  игры — для M1 (handshake/status) не нужны, понадобятся на этапе M3+.
  Данные в репозиторий не кладём (ARCHITECTURE.md, раздел 5).
- Детали протокола 26.3 не выдумываются: всё берётся из генератора данных
  и публичной документации протокола; недостающее помечать `TODO`.
