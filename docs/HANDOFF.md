# HANDOFF (state for the next session)

**M4.5 (сервер), Шаг 3.5 пересдаётся по варианту 1 (владелец)** — 12.11.2026.
- Шаг 3 готов: `crates/net/src/game/entities.rs` (`visibility_pass`, круг `vd×16`, `min(r_a,r_b)`, себя не видит, наивный O(P2)), `Player.visible`/`last_broadcast`, `TickOutcome.entities`, `kind: entity` (сорт 3, kick → 4), `ServerMessage::StreamEntities` + `deliver_entities`. Гейт зелёный (61 ok).
- Harness Шага 3.5: `crates/bench/src/visibility.rs` (+ `players::dense_cluster`/`uniform_area`), режим `cargo run -p bench --release -- visibility`; один регион inline, вход исключён, ходьба 0.25 бл/тик, 3 с прогрева + 15 с, 3 повтора, P=64/200/512.
- **Стоп-условие:** uniform P=200 установившийся p99 = 7.9–8.3 мс > 5 мс (3/3). Причина прежде — оценка; теперь пересдаётся по измерению.
- **Вариант 1, шаг 1 — инструментирование (этот коммит).** Шов в `net` для раздельного замера: `Scheduler::tick_core` (остальной тик), `Scheduler::refresh_entities` (проход видимости), `tick` = композиция; общий сборщик пакетов `crates/net/src/entity_packets.rs` (используют и `connection/play/entities.rs`, и bench); `BenchPlayer.rx` + `drain_outbound` (чтобы `try_send` не заполнял очередь). В bench добавлен режим `visibility-phases`: четыре фазы за такт — (а) видимость, (б) сборка пакетов, (в) `try_send`, (г) остальной тик; p50/p99/max, 3 повтора; uniform P=200 + dense P=64/200/512.
- Дальше по варианту 1: (2) оптимизировать то, что покажет таблица; (3) эквивалентность (эталон-наивка в тестах); (4) повтор замера, порог uniform P=200 p99 ≤ 5 мс; (5) прошёл → Шаг 4, не прошёл → таблица «до/после» + варианты и стоп.
- Изменяемое: `crates/net`, конфиг `crates/server`, `crates/protocol` (add-only), `crates/bench`, `crates/parity` (новые), `tools/`, `docs/`. `server.toml` не трогать.
- Graph: обновлён (`graphify update .`).
