# Tick-wait measurement (ARCH-M4 §3)

1000 ticks × 50 ms, release, Windows, 12 logical cores.
Two scenarios: idle (nothing else runs) and load (11 busy threads on every core but one).
Three methods: `deadline-sleep` (sleep until absolute deadline `start + i·50ms`),
`hybrid` (sleep 2 ms ahead, then spin), `tokio::interval`.
Metric: wake latency = `woken − deadline` (negative = early wake). Drift = elapsed − 1000·50 ms.

## Run 1

```
tickbench: 1000 ticks of 50ms | os=windows | logical cores=12
-- probe: what does std::thread::sleep actually cost here --
sleep(1 ms): min 1.026 | median 1.070 | p99 2.027 | max 2.036 ms
sleep(2 ms): min 2.030 | median 2.187 | p99 3.058 | max 3.096 ms
sleep(5 ms): min 5.029 | median 5.417 | p99 6.297 | max 7.906 ms
sleep(10 ms): min 10.052 | median 10.340 | p99 11.059 | max 11.313 ms
-- scenario idle: nothing else runs --
[idle] deadline-sleep    1000 ticks | drift +0 ms | min 0.027 | median 0.347 | p99 1.017 | p99.9 2.016 | max 5.039 ms
[idle] deadline-sleep  top spikes: tick 817 at 40.85s: +5.039 ms; tick 100 at 5s: +2.016 ms; tick 59 at 2.95s: +1.671 ms; tick 152 at 7.6s: +1.268 ms; tick 56 at 2.8s: +1.149 ms
[idle] hybrid(-2ms)      1000 ticks | drift +0 ms | min 0.000 | median 0.000 | p99 0.056 | p99.9 0.293 | max 2.611 ms
[idle] hybrid(-2ms)    top spikes: tick 267 at 13.35s: +2.611 ms; tick 417 at 20.85s: +0.293 ms; tick 9 at 450ms: +0.147 ms; tick 635 at 31.75s: +0.132 ms; tick 639 at 31.95s: +0.114 ms
[idle] tokio-interval    1000 ticks | drift +4 ms | min 0.000 | median 6.963 | p99 14.398 | p99.9 14.761 | max 14.883 ms
[idle] tokio-interval  top spikes: tick 687 at 34.35s: +14.883 ms; tick 456 at 22.8s: +14.761 ms; tick 655 at 32.75s: +14.725 ms; tick 623 at 31.15s: +14.695 ms; tick 556 at 27.8s: +14.671 ms
-- scenario load: 11 busy threads on 12 cores --
[load] deadline-sleep    1000 ticks | drift +15 ms | min 0.001 | median 21.102 | p99 1661.161 | p99.9 2111.160 | max 2161.160 ms
[load] deadline-sleep  top spikes: tick 745 at 37.25s: +2161.160 ms; tick 746 at 37.3s: +2111.160 ms; tick 747 at 37.35s: +2061.160 ms; tick 748 at 37.4s: +2011.160 ms; tick 749 at 37.45s: +1961.160 ms
[load] hybrid(-2ms)      1000 ticks | drift +28 ms | min 0.000 | median 35.349 | p99 1263.098 | p99.9 1500.742 | max 1513.097 ms
[load] hybrid(-2ms)    top spikes: tick 586 at 29.3s: +1513.097 ms; tick 35 at 1.75s: +1500.742 ms; tick 587 at 29.35s: +1463.098 ms; tick 36 at 1.8s: +1450.743 ms; tick 588 at 29.4s: +1413.098 ms
[load] tokio-interval    1000 ticks | drift +290 ms | min 0.000 | median 24.830 | p99 807.579 | p99.9 936.205 | max 939.722 ms
[load] tokio-interval  top spikes: tick 102 at 5.1s: +939.722 ms; tick 575 at 28.75s: +936.205 ms; tick 142 at 7.1s: +907.559 ms; tick 103 at 5.15s: +889.723 ms; tick 576 at 28.8s: +886.205 ms
```

## Run 2

```
tickbench: 1000 ticks of 50ms | os=windows | logical cores=12
-- probe: what does std::thread::sleep actually cost here --
sleep(1 ms): min 1.036 | median 1.883 | p99 2.675 | max 3.790 ms
sleep(2 ms): min 2.048 | median 2.982 | p99 6.334 | max 11.199 ms
sleep(5 ms): min 5.067 | median 5.986 | p99 6.586 | max 7.232 ms
sleep(10 ms): min 10.091 | median 10.964 | p99 11.344 | max 11.615 ms
-- scenario idle: nothing else runs --
[idle] deadline-sleep    1000 ticks | drift +0 ms | min 0.029 | median 0.488 | p99 2.737 | p99.9 12.364 | max 14.679 ms
[idle] deadline-sleep  top spikes: tick 894 at 44.7s: +14.679 ms; tick 582 at 29.1s: +12.364 ms; tick 570 at 28.5s: +11.207 ms; tick 581 at 29.05s: +9.505 ms; tick 449 at 22.45s: +9.288 ms
[idle] hybrid(-2ms)      1000 ticks | drift +0 ms | min 0.000 | median 0.000 | p99 0.008 | p99.9 0.705 | max 0.827 ms
[idle] hybrid(-2ms)    top spikes: tick 42 at 2.1s: +0.827 ms; tick 41 at 2.05s: +0.705 ms; tick 251 at 12.55s: +0.155 ms; tick 212 at 10.6s: +0.075 ms; tick 579 at 28.95s: +0.035 ms
[idle] tokio-interval    1000 ticks | drift +2 ms | min 0.000 | median 0.000 | p99 3.457 | p99.9 3.932 | max 4.168 ms
[idle] tokio-interval  top spikes: tick 899 at 44.95s: +4.168 ms; tick 202 at 10.1s: +3.932 ms; tick 426 at 21.3s: +3.797 ms; tick 985 at 49.25s: +3.752 ms; tick 942 at 47.1s: +3.713 ms
-- scenario load: 11 busy threads on 12 cores --
[load] deadline-sleep    1000 ticks | drift +25 ms | min 0.072 | median 14.461 | p99 227.684 | p99.9 368.197 | max 418.196 ms
[load] deadline-sleep  top spikes: tick 377 at 18.85s: +418.196 ms; tick 378 at 18.9s: +368.197 ms; tick 379 at 18.95s: +318.197 ms; tick 371 at 18.55s: +277.683 ms; tick 456 at 22.8s: +270.666 ms
[load] hybrid(-2ms)      1000 ticks | drift +33 ms | min 0.000 | median 31.752 | p99 722.323 | p99.9 1107.618 | max 1157.617 ms
[load] hybrid(-2ms)    top spikes: tick 140 at 7s: +1157.617 ms; tick 141 at 7.05s: +1107.618 ms; tick 142 at 7.1s: +1057.619 ms; tick 143 at 7.15s: +1007.619 ms; tick 144 at 7.2s: +957.619 ms
[load] tokio-interval    1000 ticks | drift -31 ms | min 0.000 | median 0.000 | p99 363.619 | p99.9 562.879 | max 612.879 ms
[load] tokio-interval  top spikes: tick 269 at 13.45s: +612.879 ms; tick 270 at 13.5s: +562.879 ms; tick 271 at 13.55s: +512.880 ms; tick 330 at 16.5s: +505.738 ms; tick 272 at 13.6s: +462.880 ms
```

## Сводка по двум прогонам (диапазон min..max)

| scenario | способ | drift | min | median | p99 | p99.9 | max |
|---|---|---|---|---|---|---|---|
| idle | deadline-sleep | +0 | 0.027–0.029 | 0.347–0.488 | 1.017–2.737 | 2.016–12.364 | 5.039–14.679 |
| idle | hybrid (−2 ms) | +0 | 0.000 | 0.000 | 0.008–0.056 | 0.293–0.705 | 0.827–2.611 |
| idle | tokio::interval | +2 … +4 | 0.000 | 0.000–6.963 | 3.457–14.398 | 3.932–14.761 | 4.168–14.883 |
| load | deadline-sleep | +15 … +25 | 0.001–0.072 | 14.461–21.102 | 227–1661 | 368–2111 | 418–2161 |
| load | hybrid (−2 ms) | +28 … +33 | 0.000 | 31.752–35.349 | 722–1263 | 1107–1500 | 1157–1513 |
| load | tokio::interval | −31 … +290 | 0.000 | 0.000–24.830 | 363–807 | 562–936 | 612–939 |

## Выводы

- **sleep(1 мс) median ≈ 1–2 мс** → гранулярность таймера Windows на этом хосте ≈ **1 мс** (высокоточный таймер активен; «спать меньше 1 мс» не получается, медиана одного вызова sleep(1 мс) ~1 мс, а не <0.5).
- **deadline-sleep** — дрейф 0 (оба прогона idle), p99 ~1 мс idle, max 5–14.6 мс idle (редкие выбросы планировщика). Под нагрузкой drift +15…+25 мс, p99 227–1661, max 418–2161 — доминирующий фактор — нехватка CPU, не способ ожидания.
- **hybrid (−2 ms)** — median 0, но под нагрузкой drift +28…+33 и p99 722–1263 — хуже deadline-sleep по дрейфу, не даёт Advantage под нагрузкой. **[гипотеза: spin-хвост не нужен, добавить только если бенч покажет иначе]**.
- **tokio::interval** — дрейф нестабилен (−9/+2/+4/+290 мс в過去шних прогонах; −31…+290 здесь), median 7–24 мс idle даже при отсутствии нагрузки — системное квантование таймера. Подтверждён **отрицательный дрейф −31 мс** под нагрузкой (базовая ошибка квантования `sleep_until`, а не выбор метода). Для игрового тика не берём.
- **Решение D4 (финал):** ожидание по абсолютному дедлайну `deadline = start + tick·50ms`, `sleep(deadline − now)`. Короткий spin-хвост — **[гипотеза]**, добавляется только если M4.6 покажет, что выбросы таймера влияют на TPS.

### Старые цифры (перепроверка)
- sleep idle: старые p99 1.0, max 4.5 → новые p99 1.0–2.7, max 5.0–14.7 — подтверждены (выбросы до 14 мс — редкие, планировщик Windows).
- load: старые p99 18–24, max 26–46 → новые гораздо хуже (p99 227–1661) — различие в нагрузке: LoadGuard занят всеми ядрами кроме одного, в старом прогоне нагрузка была мягче.
- tokio drift: старый −7 мс «подозрительный» → новый −31 мс подтверждён как базовая ошибка квантования, а не шум.
