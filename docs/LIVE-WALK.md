# Живой клиент на ванили (v3) через запись-прокси — инструкция

Статус: **не запускалось**, ждём решения владельца. Это инструкция, а не
автоматический скрипт; шаги выполняются вручную в PowerShell.

Цель: живой клиент проходит ходьбой по ванильной копии мира v3, а
запись-прокси на 25565 сохраняет трейс ходьбы (тот же формат JSONL, что у
бота). Зона подобрана так, чтобы **окно приёма r=10 не выходило за готовую
зону 65×65** и ваниль не генерировала новых чанков.

## Что нужно на диске

- Ваниль: `E:\MineRust-data\vanilla` (jar `E:\MineRust-data\server.jar`),
  `server.properties`: `server-port=25566`,
  `network-compression-threshold=-1`, `online-mode=false`,
  `gamemode=spectator`, `view-distance=10`.
- Шаблон v3: `E:\MineRust-data\world-template-v3` (копируется, **сам шаблон
  не трогаем**).
- Прокси-бинарь: `cargo run -p parity --bin mcproxy`.

## Запуск (по шагам)

1. Свежая копия v3 в рабочий мир ванили:
   ```powershell
   $world = 'E:\MineRust-data\vanilla\world'
   if (Test-Path $world) { Remove-Item -Recurse -Force $world }
   Copy-Item -Recurse 'E:\MineRust-data\world-template-v3' $world
   ```
2. Отключить ватчдог только на время прогона (потом вернуть):
   ```powershell
   $props = 'E:\MineRust-data\vanilla\server.properties'
   Copy-Item $props "$props.bak" -Force
   (Get-Content $props) -replace '^max-tick-time=.*', 'max-tick-time=-1' |
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
   Подключение — `127.0.0.1:25565`. JSONL ляжет как
   `E:\MineRust-data\traces\proxy-conn-*.jsonl`.
5. Зайти живым клиентом на `127.0.0.1:25565` и пройтись пешком по зоне ниже.
6. Остановить прокси (Ctrl+C), затем в консоли ванили `stop`, вернуть
   `server.properties` из `$props.bak`.

## Безопасная зона для ходьбы

- Спавн — блок (96.5, −31.5), чанк (6, −2): это **центр зоны**.
- Держать чанк игрока (F3) в прямоугольнике **x ∈ [−14, 26], z ∈ [−22, 18]**
  (по 2 чанка запаса внутри готовой зоны `x −26..38`, `z −34..30`).
  По блокам это примерно **x ∈ [−224, 431], z ∈ [−352, 303]**.
- Проще: не уходить от спавна дальше ~320 блоков по x и z.
- Пока игрок в этой зоне, окно r=10 вокруг его чанка целиком лежит в
  готовых 65×65 чанках, и ваниль новых чанков не генерирует. Выход за
  границу — не поломка, но замер уходит из подготовленной зоны (появятся
  новые `structure_starts` вне окна).

## Что смотреть в трейсе после прогона

- `python tools\trace-analyze\analyze.py window <jsonl>` — последовательность
  `set_chunk_cache_center` (время, позиция, центр) и `forget_level_chunk`.
- `python tools\trace-analyze\analyze.py outside <jsonl> --before <v3-scan>
  --after <run-scan>` — новые чанки внутри/вне окна (сканы
  `python target\parity\chunk_status.py scan <world>`).
