# HANDOFF (state for the next session)

**M4.5 (сервер), Шаг 3.5 — СТОП** — 12.11.2026. База: Шаг 3 `6c1a850`; harness замеров — этот коммит.
- Шаг 3 готов: `crates/net/src/game/entities.rs` (`visibility_pass`, круг `vd×16`, `min(r_a,r_b)`, себя не видит, наивный O(P²)), `Player.visible`/`last_broadcast`, `TickOutcome.entities`, `kind: entity` (сорт 3, kick → 4), `ServerMessage::StreamEntities` + `deliver_entities`; пакеты — `crates/net/src/connection/play/entities.rs` (порядок спавна как C1; `PLAYER_INFO_GAME_MODE = 3` по трейсу). Гейт зелёный (61 ok).
- Harness Шага 3.5: `crates/bench/src/visibility.rs` (+ `players::dense_cluster`/`uniform_area`), режим `cargo run -p bench --release -- visibility`; capacity без сети, один регион inline, вход исключён, ходьба 0.25 бл/тик, 3 прогрева + 15 с, 3 повтора, P=64/200/512.
- **Стоп-условие:** uniform P=200 установившийся p99 = 7.9–8.3 мс > 5 мс (3/3 повторов). dense P=200 p99 21–27 мс, dense P=512 p99 155–204 мс, uniform P=512 p99 43–52 мс.
- Оценка причины: наивный проход, но на каждом O(P²)-сравнении 4–6 обращений к `BTreeMap` и перестройка `BTreeMap`/`BTreeSet` + `name().to_owned()` каждый тик → фактически O(P² log P) с большими константами. Варианты — в `docs/PROGRESS.md` раздел «Блокеры» (рекомендую п.1: снимок в `Vec`, индекс id→i, без клонов в горячем пути).
- Шаг 4 (тесты а–д) **не начат** (стоп по плану). Ждём решение владельца.
- Изменяемое: `crates/net`, конфиг `crates/server`, `crates/protocol` (add-only), `crates/bench`, `crates/parity` (новые), `tools/`, `docs/`. `server.toml` не трогать.
- Graph: обновлён (`graphify update .`).