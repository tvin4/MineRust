# BLOCK-PLACING.md — постановка блоков в творческом режиме: данные ванили и план (Шаг 23d)

Статус: **только анализ и план, кода сервера нет** (условие 23d). Источник —
живые ванильные S17-трейсы; наш сервер в 23d не запускался, реализации
`use_item_on` у нас нет.

## 1. Источник и метод

- Ванильные прогоны: `E:\MineRust-data\autotests\2026-11-18_s17-vanilla-step23-r1`
  (движок прогона подвис на чтении трейсов — данные на диске годны),
  `...s17-vanilla-step23-r2` — все 13 per-case проверок прошли, но
  пост-обработка трейсов вернула 0 строк (полнофайловое чтение двух больших
  трейсов), и `...s17-vanilla-step23-r3` — 0 failed после перехода харнесса на
  построчное стриминговое чтение (`Read-TraceMatches`); артефакт
  `s17-samples.json` полный.
- Сценарий: `tools/testclient/scenarios/s17-creative-place.json`, 14 кейсов.
  Стенд: ванильный сервер 26.3, два Fabric-тест-клиента A/B, `gamemode creative`,
  `forceload 70 -50 170 5`. Фикстура ставится командами `setblock`; бой
  (`use`) посылает `-KeyCmd use tap`.
- Разбор байтов: `E:\MineRust-data\traces\s17_decode.py` (wire-декодеры) и
  `s17_facts.py` (построитель таблиц, читает `reports/blocks.json` и
  `reports/registries.json`). Скрипты вне репозитория.
- ID блоков/предметов/пакетов — из `reports/blocks.json`,
  `reports/registries.json`, `reports/packets.json` (источники истины 1).

Геометрия кейса (одинакова для всех, кроме отмеченных): игрок стоит в
`(x+0.5, 201, −24.5)` взглядом `yaw=0, pitch=2`; `support` — блок в
`(x, 202, −20)`; `dest` — `(x, 202, −21)`, т.е. на северной (`−Z`) грани
`support`; `source` — блок для creative-pick в `(x, 200, −40)`. То есть клиент
бьёт по **северной** грани (`face=2`) и ставит блок на клетку `z = support.z − 1`.

## 2. (а) Что клиент присылает при постановке (serverbound)

Наблюдённый порядок на кейс: `pick_item_from_block` → серверный ответ
(`container_set_slot` + `set_held_slot`) → `use_item_on` (постановка). Выбор
слота в хотбаре — отдельным `set_carried_item` перед следующим кейсом.

| пакет | id (`packets.json`) | поля | наблюдено (r1) |
|---|---:|---|---:|
| `set_carried_item` | 54 | `slot`: i16 (big-endian) | 8 |
| `pick_item_from_block` | 36 | `pos` (packed), `include_data`: bool | 14 |
| `use_item_on` | 66 | `hand` varint, `pos` packed, `face` varint, `cursor_x/y/z` f32, `inside` bool, `world_border` bool, `sequence` varint | 11 |
| `use_item` | 67 | `hand` varint, `sequence` varint, `yaw` f32, `pitch` f32 | 3 |
| `interact` | 26 | `entity` varint, `action` varint (+ вектор/рука для `action ∈ {0,2}`) | 2 |
| `set_creative_mode_slot` | 57 | `slot` i16, item stack | **0** |

Выводы для нашей реализации:

- **`set_creative_mode_slot` в творческом pick-флоу 26.3 не участвует** (0
  пакетов, проверка «observed, not required» в раннере). Pick отправляет
  `pick_item_from_block`; сервер сам кладёт предмет в хотбар.
- Постановка — исключительно `use_item_on` (промах мимо блока → `use_item`
  без `pos`). `interact`/`swing` к постановке блоков не относятся.
- Все записи `use_item_on` в r1: `face=2` кроме последнего кейса (`face=5`),
  `hand=0`, `inside=false`, `world_border=false`, `sequence` 1..12.

## 3. (б) Результирующий state id: «блок → state id → из чего вычисляется»

`dest state id` — фактический блок в мире после `use` (joined с `blocks.json`).
«Из чего» — из каких входных данных ваниль вывела это состояние.

| кейс | ставим | dest state id | свойства (`blocks.json`) | из чего вычисляется |
|---|---|---:|---|---|
| `stone` | `minecraft:stone` | 1 | `{}` | единственное состояние блока, геометрия не влияет |
| `dirt` | `minecraft:dirt` | 10 | `{}` | то же |
| `planks` | `minecraft:oak_planks` | 15 | `{}` | то же |
| `glass` | `minecraft:glass` | 661 | `{}` | то же |
| `log-face` | `minecraft:oak_log` | 141 | `axis=z` | **ось грани клика** `face=2` (нормаль `−Z`) → `axis=z`; `x=139`, `y=140` |
| `slab-low-click` | `minecraft:oak_slab` | 15180 | `type=bottom, waterlogged=false` | `cursor_y=0.2264 < 0.5` → `bottom` (верхний — `15178`) |
| `slab-high-click` | `minecraft:oak_slab` | 15178 | `type=top, waterlogged=false` | `cursor_y ≥ 0.5` → `top` (подтверждено в r2) |
| `stairs-south-view` | `minecraft:oak_stairs` | 5494 | `facing=south, half=bottom, shape=straight, waterlogged=false` | `facing` ← горизонтальный `yaw` (0 = на юг), `half` ← `cursor_y`, `shape` ← соседи (тут `straight`), `waterlogged` ← вода |
| `stairs-west-view` | `minecraft:oak_stairs` | 5514 | `facing=west, half=bottom, shape=straight, waterlogged=false` | `facing` ← `yaw=90` (на запад) |
| `replace-short-grass` | `minecraft:stone` | 1 | `{}` | результат замены, см. §4 |
| `replace-snow` | `minecraft:stone` | 1 | `{}` | то же |
| `replace-water` | `minecraft:stone` | 1 | `{}` | то же |

Порядок `facing` в ID `oak_stairs` — `north, south, west, east`, по 20 состояний
на грань (10 `half=top`, затем 10 `half=bottom`; на полушаг 5 `shape` × 2
`waterlogged`). Поэтому `south/bottom/straight/false = 5494`,
`west/bottom/straight/false = 5514` — совпадает с трейсом.

Правило геометрии (`SOURCE: S17 vanilla r1`):

- ось/направление по нормали грани: `log.axis` (x — грани `4/5`, y — `0/1`,
  z — `2/3`);
- `stairs.facing` по горизонтальному взгляду игрока (кардинал), `half` по
  `cursor_y`, `shape` по форме соседних лестниц, `waterlogged` по воде;
- `slab.type` по `cursor_y` (`< 0.5` → `bottom`, `≥ 0.5` → `top`);
- остальные блоки — единственное состояние по умолчанию.

Смещение `tail`-половины лестниц (`shape`) и `waterlogged` в S17 не
варьировалось (соседи/вода не готовились) — **не установлено**, отмечаем как
долг.

## 4. (в) Замена существующего блока

Кейсы `replace-short-grass`/`replace-snow`/`replace-water` готовят в `dest`
заменяемый блок (`short_grass`/`snow[layers=1]`/`water`) поверх `surface`
(`dirt`) и ожидают после боя `minecraft:stone`. В обоих прогонах `dest` после
боя = `state 1` (`minecraft:stone`): ваниль **заменила** целевой блок одним
`use_item_on`, без отдельного `player_action`/ломания.

В r2 (фикстура уточнена ключом `surface`) предпроверка прошла: A и B видят в
`dest` именно `short_grass`/`snow`/`water` до боя, после боя — `stone`
(14/14 кейсов PASS). Поэтому замена подтверждена и по исходному, и по
результирующему блоку. В r1 предпроверка `replace-short-grass` падала —
трава без опоры ломалась при `setblock`-обновлении соседа; это был дефект
фикстуры, не поведение сервера.

Кейс `slab-high-click` в r1 промахнулся (клиент послал `use_item`, постановки
не было); в r2 питч взгляда поправлен — поставлена верхняя плита (`state 15178`).

### 4.1 Куда именно кладётся при замене (уточнено в 24c)

В r2 `replace-short-grass` посылает `use_item_on` **по самой траве**:
`pos=(104,202,-21)`, `face=2`, `cursor=[0.5, 0.4934, 0.125]`,
`sequence=5` (seq 80534), а не по опоре за ней. Причина — `short_grass`
таргетабельна, и луч останавливается на её контуре (§7 стадия 3 предполагала
обратное). Ответ автора: `block_update` `(104,202,-21)=1`,
`block_update` `(104,202,-22)=0`, `block_update` `(104,202,-21)=1`, затем
`block_changed_ack` `sequence=5`; наблюдатель — один `block_update`
`(104,202,-21)=1`.

Отсюда правило ванили для целевой клетки: если состояние в кликнутой клетке
заменяемо (`air`, `water`, `short_grass`, `snow`), ставим **в неё**, иначе — в
клетку по нормали грани. `23d`-описание «замена одним `use_item_on`» верно,
но «цель = клик + нормаль» — только для незаменяемой кликнутой клетки.

## 5. (г) Инвентарь творческого режима и `.dat`

Clientbound (окно `0`, `state` растёт 1,2,3…):

| пакет | id | наблюдено | смысл |
|---|---:|---:|---|
| `container_set_content` | 18 | 2 | при входе: 46 пустых слотов, `carried` пусто |
| `container_set_slot` | 20 | 7 | creative-pick кладёт предмет в слот `36..42` |
| `set_held_slot` | 107 | 16 | выбранный слот хотбара (эхо/выбор) |
| `set_cursor_item` | 98 | 0 | не участвует |

Слот `36` — первый слот хотбара игрока; pick первого блока кладёт `stone`
(item id `1`) в `36`, второго — `dirt` (`55`) в `37`, и т.д. до `oak_stairs`
(`515`) в `42`. Предметные protocol id — из `registries.json`
(`minecraft:item.entries[*].protocol_id`).

Файл игрока (`players/data/<uuid>.dat`, прочитан read-only дампером):
`playerGameType=1` (creative), `SelectedItemSlot=0`, `DataVersion=5023`,
`Inventory` — 7 записей, `Slot 0..6` = `stone, dirt, oak_planks, glass,
oak_log, oak_slab, oak_stairs` (по одному). То есть путь
`pick → хотбар → Inventory` доходит до диска.

У нас **нет модели инвентаря**: сервер не шлёт `container_set_*`/
`set_held_slot` и не отвечает на `pick_item_from_block`/`set_carried_item`
(проверено grep по `crates`). `playerdata` сохраняет незнакомые теги, включая
`Inventory`, но игрового смысла им не придаёт.

## 6. (д) Связь с `ChunkStore` / dirty / свет

Что уже есть (образец — путь ломания, `crates/net/src/game/break_block.rs`):

- `ChunkStore::set_block(x,y,z, state)` пишет клетку, помечает чанк грязным и
  возвращает прежний state; правки уходят в `worldsave` диффами `.mrchunk`
  (`WORLD-SAVE.md`); `ChunkStore::block`/`chunk_cell_state` — чтение.
- Наблюдателям шлётся `ServerMessage::BlockUpdate { position, state }`
  (`block_update`, id 8) каждому игроку региона; автору — `BlockChangedAck`
  (id 4) с `sequence`. `level_event` — только для ломания.
- `ToGame`/`ServerMessage` — единственный путь (сообщения, без `Mutex` в тике).

Чего нет и что потребуется:

- **свет**: при постановке/ломании блоков свет мы не пересчитываем и
  `light_update` не шлём; ваниль пересчитывает его и рассылает обновления
  освещения. В S17-кейсах (творческий режим, плоская площадка в 200) свет
  менялся мало; отдельного наблюдения `light_update` в разборе **нет** —
  **не установлено**, вынести в план отдельной стадией/вопросом.
- **высотные карты** (`heightmaps`): постановка меняет `MOTION_BLOCKING`/
  `WORLD_SURFACE`; у нас они лежат в шаблоне и не обновляются.
- **коллизия/поза**: стадии поз читают `reports/collision.json` из шаблона,
  динамическая поставка блока в рантайме не меняет коллизии (долг из
  `SWIMMING.md`); постановка поставит и его.
- **блок-сущности/предметы**: вне темы (см. долги `WORLD-SAVE.md`).

## 7. (е) Стадии реализации (план, риски, размер)

Порядок — снизу вверх; каждая стадия отдельный коммит и живой прогон.

1. **Инвентарь/хотбар (модель + join-пакеты).** Новое состояние игрока:
   46 слотов окна `0`, выбранный слот; `container_set_content` при входе,
   `set_held_slot`; приём `set_carried_item` (id 54); сохранение в `Inventory`
   через `playerdata`. Риск: трогает join-последовательность (7→8 пакетов) —
   сломать S13/S15/S16; кодеки `container_*` нет. Размер: L.
2. **`pick_item_from_block` (36) → ответ.** Резолв блока в `pos` в предмет
   (`reports` `block → item`), поиск слота (совпадающий стек / свободный),
   `container_set_slot` (20) в хотбар-слот + `set_held_slot` (107). Риск:
   правило выбора слота ванили точно не снято (гипотеза «первый пустой» по
   r1, фиксированная последовательность) — подтвердить трейсом. Размер: M.
3. **`use_item_on` (66): разбор и цель.** Декод (hand/pos/face/cursor/inside/
   world_border/sequence), проверка дистанции (творческая
   `block_interaction_range` 4.5+0.5 — см. 22a.3), выбор целевой клетки по
   грани/`inside`, проверка «занято игроком/нельзя». Риск: правило «куда
   кладётся при `inside`/по верхней грани» — 5-я грань в r1 не разобрана.
   Размер: M.
4. **Резолв состояния и запись.** Таблица §3 (log/slab/stairs/вода + default),
   `ChunkStore::set_block`, `BlockUpdate` наблюдателям + `BlockChangedAck`
   автору, замена целевого блока (§4) без отдельного ломания. Риск: геометрия
   лестниц (`shape`/`waterlogged`) и свет — вне первых стадий. Размер: M.
5. **Живой S17 на нашем сервере + parity + `.dat`.** `-Target ours`,
   инвентарь после pick в `save_dir`; при желании — сравнение с ванилью
   (пакетный трейс). Риск: нужен стенд владельца; расхождения временны
   (timing). Размер: M.

Итого грубо: 1×L + 4×M. Блокеров-архитектурных нет; главные риски — join-
последовательность (стадия 1) и неполные правила ванили (стадии 2–4).
Свет/высотные карты/коллизии — **отдельный** вопрос вне этого плана.

## 8. Вопросы владельцу

1. Нужен ли инвентарь/хотбар вообще (стадия 1), или до block entities/items
   достаточно только «поставить блок»? Без инвентаря `use_item_on` нечем
   проверять «в руке».
2. Порядок: сначала стадия 1 (инвентарь) или сначала «поставить блок по
   серверному `setblock`-эквиваленту» (стадии 3–4) без инвентаря?
3. Свет/высотные карты (`light_update`, `heightmaps`) — обязательны в 23d-план
   или отдельным шагом? В S17-трейсах `light_update` не разбирался.
4. Творческий лимит взаимодействия: брать `block_interaction_range`
   4.5 + `creative_mode_block_range` 0.5 (как в 22a.3) или иное правило?
5. Формат сравнения с ванилью для S17 (пакетный трейс побайтно или только
   «итоговый state id в мире»)?

## 9. Ответы владельца и реализация 24a–24d

Ответы (18.11.2026): (1) инвентарь нужен; (2) сначала инвентарь, потом
постановка; (3) свет — отдельно, только данные; (4) 4.5 + 0.5 = 5.0 в
креативе; (5) приёмка — побайтные пакеты **и** итоговый state id.

### 9.1 Что реализовано

- **24a.** Модель окна `0` на 46 слотов в `crates/net/src/game/inventory.rs`
  (hotbar `0..8`, main `9..35`, armor/offhand `36..44`, carried `45`),
  `container_set_content` + `set_held_slot` в join, приём `set_carried_item`,
  `pick_item_from_block` → `container_set_slot` + `set_held_slot`, приём
  `set_creative_mode_slot` (57) без clientbound-ответа (так в 24.0a).
- **24b.** `use_item_on` (`crates/net/src/game/placing.rs`): целевая клетка =
  кликнутая, если она заменяема (§4.1), иначе клик + нормаль грани; предел
  5.0 в креативе / 4.5 иначе; отказ, если блок пересекает бокс игрока; резолв
  состояния по таблице §3; ответ автору — `block_update` (клик), `block_update`
  (предсказанный сосед), `block_update` (цель), `block_changed_ack`; наблюдателям
  один `block_update`.
- **24d.** `Inventory` (список `SavedItem` + `SelectedItemSlot`) читается и
  пишется в `<save_dir>/players/data/<uuid>.dat` (`crates/playerdata`).

### 9.2 S17: наш сервер против ванили, побайтно

Прогоны: ваниль `2026-11-18_s17-vanilla-step23-r2`, наш
`2026-11-18_s17-ours-step24-r4` (наш клиент, dev-геометрия в `y≈200`,
творческий режим на входе, `tp` командой тест-мода вместо `/tp`). Обе стороны —
mcproxy. Ниже — байты **без** номеров seq прокси (номера различаются по
построению). В наших строках `use_item_on` побайтно равен ванильному, включая
`sequence` 1..12, `face` и `cursor`.

| # | кейс | `use_item_on` (hex) | `block_update` автору по порядку (hex) | ack | наблюдатель | state id |
|---:|---|---|---|---|---|---:|
| 1 | stone | `000000143ffffec0ca023f0000003eed009100000000000001` | `0000143ffffec0ca01`, `0000143ffffeb0ca01`, `0000143ffffeb0ca01` | `01` | `0000143ffffec0ca01` | 1 |
| 2 | dirt | `00000015bffffec0ca023f0000003eed009100000000000002` | `000015bffffec0ca01`, `000015bffffeb0ca0a`, `000015bffffeb0ca0a` | `02` | `000015bffffeb0ca0a` | 10 |
| 3 | planks | `000000173ffffec0ca023f0000003eed009100000000000003` | `0000173ffffec0ca01`, `0000173ffffeb0ca0f`, `0000173ffffeb0ca0f` | `03` | `0000173ffffeb0ca0f` | 15 |
| 4 | glass | `00000018bffffec0ca023f0000003eed009100000000000004` | `000018bffffec0ca01`, `000018bffffeb0ca9505`, `000018bffffeb0ca9505` | `04` | `000018bffffeb0ca9505` | 661 |
| 5 | replace-short-grass | `0000001a3ffffeb0ca023f0000003efca4953e000000000005` | `00001a3ffffeb0ca01`, `00001a3ffffea0ca00`, `00001a3ffffeb0ca01` | `05` | `00001a3ffffeb0ca01` | 1 |
| 6 | replace-snow | `0000001bbffffec0ca023f0000003eed009100000000000006` | `00001bbffffec0ca01`, `00001bbffffeb0ca01`, `00001bbffffeb0ca01` | `06` | `00001bbffffeb0ca01` | 1 |
| 7 | replace-water | `0000001d3ffffec0ca023f0000003eed009100000000000007` | `00001d3ffffec0ca01`, `00001d3ffffeb0ca01`, `00001d3ffffeb0ca01` | `07` | `00001d3ffffeb0ca01` | 1 |
| 8 | log-face | `0000001ebffffec0ca023f0000003eed009100000000000008` | `00001ebffffec0ca01`, `00001ebffffeb0ca8d01`, `00001ebffffeb0ca8d01` | `08` | `00001ebffffeb0ca8d01` | 141 |
| 9 | slab-low-click | `000000203ffffec0ca023f0000003e67d4c800000000000009` | `0000203ffffec0ca01`, `0000203ffffeb0cacc76`, `0000203ffffeb0cacc76` | `09` | `0000203ffffeb0cacc76` | 15180 |
| 10 | slab-high-click | `00000021bffffec0ca023f0000003f46f0640000000000000a` | `000021bffffec0ca01`, `000021bffffeb0caca76`, `000021bffffeb0caca76` | `0a` | `000021bffffeb0caca76` | 15178 |
| 11 | stairs-south-view | `000000233ffffec0ca023f0000003eed00910000000000000b` | `0000233ffffec0ca01`, `0000233ffffeb0caf62a`, `0000233ffffeb0caf62a` | `0b` | `0000233ffffeb0caf62a` | 5494 |
| 12 | stairs-west-view | `00000024bffffec0ca053f8000003efee0953f00000000000c` | `000024bffffec0ca01`, `000024fffffec0ca8a2b`, `000024fffffec0ca8a2b` | `0c` | `000024fffffec0ca8a2b` | 5514 |
| 13 | occupied-by-player | — | — | — | — | 0 |
| 14 | out-of-range | — | — | — | — | 0 |

Проверка: `s17_compare_rows.py` сравнивает 105 строк обоих прогонов после
маскирования seq/имени трейса — **104 строки данных совпали побайтно**,
различается только строка-заголовок (путь прогона и полный размер трейса).
Наш прогон r4 — 0 failed (104 проверки). Кейсы 13/14 в обоих прогонах не
посылают `use_item_on`: клиент останавливает луч на игроке (`interact`) или
промахивается (`use_item`); разделение serverbound в обоих прогонах
одинаковое — `use_item_on` 12, `use_item` 2, `interact` 2. Итоговые state id
в мире совпали у A и B.

`set_carried_item` у нас 10 против 8 у ванили, `set_creative_mode_slot` 1 против
0 — это харнесс: в нашем прогоне дополнительно проверяется живой путь меню
креатива (24.0a → id 57, ответа нет) и выбор клавишей хотбара. Пик-флоу
(`pick_item_from_block` 14) одинаков.

### 9.3 Найденный разрыв join: собственные атрибуты игрока

Первый прогон (`2026-11-18_s17-ours-step24-r1`) не поставил ни одного блока:
у нашего клиента предел оставался 4.5, луч заканчивался в 4.5 от глаз
(`z=-20.0027` при опоре в `z=-20.5`), клиент присылал `MISS` и не отправлял
`use_item_on` (в трейсе 2 вместо 12).

Причина — в join не было `update_attributes` **своего** игрока. Ваниль
(`2026-11-18_s17-vanilla-step23-r2`, conn-1) шлёт его на 3.04 с (seq 320,
entity 76, три атрибута 26/13/8 = 0.1/3.0/4.5, без модификаторов), а при
переключении в креатив — снова (seq 62634, entity 76, два атрибута с
`creative_mode_entity_range +2.0` и `creative_mode_block_range +0.5`). У нас
`update_attributes` приходил только от `push_spawn` **другого** игрока
(entity 2 в том прогоне), поэтому клиент его игнорировал, и креативный
предел не включался.

Исправление: `send_join` отправляет `UpdateAttributes` для собственного
`entity_id` сразу после `Set Health` (8-й пакет join-последовательности; у
ванили он позже, среди первых батчей чанков — точное место не установлено,
помечено в коде). Форма атрибутов — та же `spawn_attributes`, что у
`push_spawn`, поэтому в креативе она несёт оба модификатора. После правки
S17 r4 поставил все 12 блоков (см. §9.2).

**Не установлено:** шлёт ли ваниль `update_attributes` ещё раз при смене
режима уже в игре (`set_gamemode`) и есть ли у нас такая правка — сейчас наш
сервер на смену режима собственных атрибутов не пересчитывает; проверено
только «вход сразу в креативе».

### 9.4 S18: поставленные блоки переживают graceful stop и рестарт

Прогон `2026-11-18_s18-ours-step24-r2` — **24/24 PASS**, 0 failed (стенд S16a:
stop/restart в середине сценария). Три блока поставлены живым путём
(`creative` → клавиша хотбара → `use`): `stone`=1, `oak_log`=`axis=z`=141,
`oak_stairs`=`facing=south,half=bottom,shape=straight`=5494. После graceful stop
(`exited=True exitCode=0`, `final world save flushed`, `player saves flushed`)
и рестарта тем же `save_dir` оба клиента (A и B) читают те же state id и
свойства; три слота хотбара A лежат в `players/data/<uuid>.dat`.

**Артефакт харнесса (r1, 21/24):** dev-геометрия применяется на каждом старте
**после** загрузки сейва и сбрасывала `dest` в `initial`, поэтому gen-1 читала
`air`. Что мир при этом сохранился, видно в логе gen-2 r1: `applied a saved
chunk diff` (chunk `3,-5` cells=1, `4,-5` cells=2) до строки про геометрию.
Раннер теперь на перезапуск пишет платформенную геометрию (только пол под
стойкой) и отдельный `server-ours-restart.toml`; проверка `the restart geometry
keeps only the stand floor` — 3 строки, 0 `setblock`.
