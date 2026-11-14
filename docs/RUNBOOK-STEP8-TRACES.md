# Шаг 8, Шаг 0 — runbook записи ванильных трейсов сценариев S1–S4

Статус: **инструкция, выполняется владельцем**. Код не менялся; runbook и
скрипт разбора — это подготовка, а не реализация. Цель — получить ванильные
трейсы, по которым затем (по решению владельца) будут писаться кодировщики
пакетов наблюдателю (`add_entity`, `set_entity_data`, `animate`).

Формат записи — тот же, что у `mcproxy`: JSONL на соединение,
`t_ms, seq, dir, id, name, state, b64`. Нужен **clientbound-поток наблюдателя
B** — второй клиент, который смотрит на игрока A.

## 0. Вода перепроверена по NBT (сделано, валидация)

Колонка воды (103, −32) лежит в чанке (6, −2) — **том же чанке, что и спавн**
(96, 145, −32), значит входит в готовую зону R=10 = 473 чанков. По region-NBT
вокруг (103, −32) водоём глубиной до 5 блоков (колонки x=105..107, z=−28..−32,
вода y=37..41). Для плавания рекомендуемая точка — **(106, −31), y=41**
(максимальная глубина, 5 блоков воды).

Замечание: в `blocks.json` вода — id состояний 89..104 (свойство `level`).
Скрипт проверки: `C:\Users\Tvinch\AppData\Local\Temp\opencode\verify_water_depth.py`
(раскладка ячеек — по `crates/protocol/src/packets/chunk_payload.rs`).

## 1. Что нужно на диске

- Ваниль: `E:\MineRust-data\vanilla` (jar `E:\MineRust-data\server.jar`),
  `server.properties`: `server-port=25566`,
  `network-compression-threshold=-1`, `online-mode=false`,
  **`gamemode=survival`** (в отличие от LIVE-WALK.md — там spectator),
  `view-distance=10`. `force-gamemode=true` рекомендуется (чтобы A не
  оказался в режиме по умолчанию).
- Шаблон v3: `E:\MineRust-data\world-template-v3` (**сам шаблон не трогаем**).
- Прокси-бинарь: `cargo run -p parity --bin mcproxy`.

**Важно про `max-tick-time`:** ватчдог (`max-tick-time`, по умолчанию 60000 мс)
может убить сервер при долгой паузе. Оставить `max-tick-time=-1` на время
прогона (см. LIVE-WALK.md шаг 2).

## 2. Запуск (по шагам)

1. Свежая копия v3 в рабочий мир ванили (сам шаблон не меняем):
   ```powershell
   $world = 'E:\MineRust-data\vanilla\world'
   if (Test-Path $world) { Remove-Item -Recurse -Force $world }
   Copy-Item -Recurse 'E:\MineRust-data\world-template-v3' $world
   ```
2. Временно отключить ватчдог и выставить survival (вернуть после прогона):
   ```powershell
   $props = 'E:\MineRust-data\vanilla\server.properties'
   Copy-Item $props "$props.bak" -Force
   (Get-Content $props) -replace '^max-tick-time=.*', 'max-tick-time=-1' `
       -replace '^gamemode=.*', 'gamemode=survival' |
       Set-Content $props -Encoding ascii
   ```
3. Поднять ваниль на 25566:
   ```powershell
   Start-Process java -WorkingDirectory 'E:\MineRust-data\vanilla' `
       -ArgumentList '-Xmx1G','-jar','E:\MineRust-data\server.jar','--nogui'
   ```
   Дождаться в `E:\MineRust-data\vanilla\logs\latest.log` строки `Done (...)`.
4. Поднять запись-прокси 25565 → 25566:
   ```powershell
   cargo run -p parity --bin mcproxy -- --listen 127.0.0.1:25565 --out E:\MineRust-data\traces
   ```
   Прокси откажется работать при включённом сжатии или `online-mode=true`.
   Каждое соединение пишет свой файл `proxy-conn-<N>.jsonl`:
   **N=1 — клиент A (первый вошедший), N=2 — наблюдатель B (второй)**.

## 3. Сценарии (каждый — отдельный прогон, отдельная пара файлов)

Пауза между любыми действиями — **≥ 3 с**. Между сценариями — перезапуск
ванили и новая копия мира (шаги 1–4), чтобы трейсы не смешивались.

- **S1 (A, поворот):** A заходит, поворачивается на ненулевой yaw/pitch
  (не 0°, не 90° — например, на 30–40°) и стоит ≥ 3 с. Только потом заходит B
  (наблюдатель). Смотреть в трейсе B: `add_entity` игрока A — какие байты углов
  приносит ваниль, и `rotate_head`.
- **S2 (Shift):** A стоит перед камерой B, нажимает Shift (присесть) и держит
  ≥ 3 с, отпускает, ждёт ≥ 3 с. Повторить 2–3 раза. Смотреть в трейсе B:
  `set_entity_data` (индекс/тип/значение позы).
- **S3 (взмахи + ломание):** A стоит перед камерой B, делает **три взмаха**
  ЛКМ в воздух (пауза между взмахами ≥ 3 с), затем ломает один блок
  (пауза ≥ 3 с). Смотреть в трейсе B: `animate`, `entity_event`.
- **S4 (спринт + вода):** A спринтует по земле ≥ 3 с, затем `/tp` в воду
  (106, 41, −31) и плывёт спринтом ≥ 3 с. Смотреть в трейсе B: `animate`,
  `set_entity_data` (поза плавания), `update_attributes`.

**Внимание по S1:** вход с ненулевым поворотом зависит от того, с каким
yaw/pitch клиент был при выходе из прошлой сессии (сохраняется в
`playerdata/*.dat` копии мира). Убедитесь, что A входит именно с ненулевым
углом; если вошёл с 0 — выйти и зайти после поворота.

## 4. Останов и сверка sha256

1. Остановить прокси (Ctrl+C), затем в консоли ванили `stop`.
2. Вернуть `server.properties` из `$props.bak`.
3. Сверить копию мира с шаблоном (манифест в формате `sha256  путь`, как
   `E:\MineRust-data\world-template-v1.sha256.txt`):
   ```powershell
   Get-ChildItem -Recurse -File 'E:\MineRust-data\vanilla\world' |
       ForEach-Object {
           $rel = $_.FullName.Substring('E:\MineRust-data\vanilla\world\'.Length)
           $h = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
           "$h  $rel"
       } | Sort-Object { ($_ -split '  ',2)[1] } |
       Set-Content 'E:\MineRust-data\vanilla\world.sha256.txt' -Encoding ascii
   ```
   Сверить с шаблоном: `fc E:\MineRust-data\vanilla\world.sha256.txt <манифест-шаблона>`
   либо вручную по `Get-FileHash` нескольких ключевых файлов. **Шаблон v3
   не должен меняться.**

## 5. Что делает скрипт разбора

`tools\trace-analyze\m45-step80-scenarios.py` — по трейсу наблюдателя B
печатает для каждого сценария clientbound-пакеты, относящиеся к игроку A, с
временем и полями:

- `add_entity` (id=1): три байта углов (порядок между ними — **не установлено**,
  см. `crates/protocol/src/packets/entity.rs`), `head_yaw`, `data`, `tail`;
- `rotate_head`: байт `head_yaw`;
- `set_entity_data` (id=101): записи метаданных (индекс, тип, значение + сырой
  hex), терминатор `0xFF`;
- `animate` (id=2) и `entity_event` (id=34): сырые байты + первые два VarInt
  как подсказка (**раскладка не установлена** — `packets.json` даёт только
  `protocol_id`, имён полей нет);
- `update_attributes` (id=134): ключи/базовые значения/модификаторы.

Запуск (сценарий — один из s1..s4, либо `--all`):
```powershell
python tools\trace-analyze\m45-step80-scenarios.py --dir E:\MineRust-data\traces --scenario s1
```
Файлы ожидаются как `<dir>\<сценарий>-A.jsonl` и `<dir>\<сценарий>-B.jsonl`
(переименуйте `proxy-conn-1.jsonl` → `s1-A.jsonl`, `proxy-conn-2.jsonl` →
`s1-B.jsonl`). Игрок A определяется автоматически как единственная сущность
типа 159 (`registries.json`) в трейсе B; если таких несколько (respawn) —
печатаются все.

**Метки полей:** берутся из кодеков `crates/protocol/src/packets/*` (это
единственный источник имён полей, `packets.json` их не содержит) и помечены
там, где семантика не установлена. Всё, что не подтверждено трейсом, скрипт
печатает как сырые байты с пометкой «не установлено» и не додумывает.

## 6. Что НЕ покрыто этим runbook-ом

- `player_command`/`player_input`/`punch` (serverbound) — скрипт смотрит
  clientbound-поток B. Для serverbound-анализа нужен отдельный проход (по
  `live-session.json` уже известно: `punch` id 46, `player_input` id 43).
- Crawling (поза ползания) — в список долгов, сценарий не входит.
- Тест (в) не менялся; код писать только после разбора трейсов по плану шага 8.
