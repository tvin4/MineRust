#!/usr/bin/env python
"""M4.5 step 11: the vanilla swing rule, checked against the S3 trace.

The rule is a hypothesis taken from vanilla memory and confirmed by this
model: on a server tick, a `punch` packet is accepted (and the swing is
broadcast to observers) when the player is not swinging, or the swing just
started (`swing_time < 0`), or the previous swing is old enough
(`swing_time >= threshold`). An accepted punch sets `swinging = True` and
`swing_time = -1`; at the end of the tick `swing_time += 1`, and at
`swing_time >= duration` the swing ends (`swinging = False`,
`swing_time = 0`). Packets are processed BEFORE the end-of-tick increment.

The script re-reads the raw S3 traces, joins the A/B clocks through the
`mcproxy-err.log` anchors (see docs/PROGRESS.md, "Сведение часов A/B"),
maps the punch moments onto a 50 ms tick grid over a phase/delay grid, and
reports which grid combinations reproduce the swing cluster structure of
the observer (B). The threshold itself is NOT determined by the trace:
both 3 and 4 ticks give the same structure.
"""

import base64
import json
import re
from datetime import datetime
from pathlib import Path

ROOT = Path(r"E:\MineRust-data\autotests")
FOLDER = "2026-11-14_1644"  # S3
CONN_A = 6  # punch / player_action (player A)
CONN_B = 7  # swing_animation / block_destruction (observer B)
TICK_MS = 50.0
CLUSTER_GAP_MS = 150.0  # swings closer than this belong to one cluster
PHASE_STEP_MS = 0.5
DELAYS_MS = [0, 5, 10]
THRESHOLDS = [3, 4]  # ticks; the trace does not distinguish them
DURATION = 6  # ticks (vanilla memory hypothesis)


def strip_ansi(text: str) -> str:
    return re.sub(r"\x1b\[[0-9;]*m", "", text)


def anchors(folder: str) -> dict[int, float]:
    """Wall-clock (ms since epoch, UTC) of every connection's recording start."""
    log = ROOT / folder / "mcproxy-err.log"
    out = {}
    for line in strip_ansi(log.read_text(encoding="utf-8", errors="replace")).splitlines():
        m = re.match(
            r"(\S+Z)\s+INFO\s+\S+:\s+recording\s+.*proxy-conn-(\d+)\.jsonl", line.strip()
        )
        if m:
            stamp = datetime.fromisoformat(m.group(1).replace("Z", "+00:00"))
            out[int(m.group(2))] = stamp.timestamp() * 1000.0
    return out


def rows(folder: str, conn: int) -> list[dict]:
    path = ROOT / folder / "mcproxy" / f"proxy-conn-{conn}.jsonl"
    return [json.loads(x) for x in path.open(encoding="utf-8")]


def swing_clusters(times: list[float]) -> list[int]:
    """Cluster the observed swing times; return the per-cluster sizes."""
    clusters = []
    for t in times:
        if clusters and t - previous < CLUSTER_GAP_MS:
            clusters[-1] += 1
        else:
            clusters.append(1)
        previous = t
    return clusters


def model_clusters(punch_ms: list[float], phase: float, delay: float, threshold: int) -> list[int]:
    """Apply the tick model; return the accepted-per-tick cluster sizes."""
    ticks: dict[int, int] = {}
    for t in punch_ms:
        index = int((t + phase + delay) // TICK_MS)
        ticks[index] = ticks.get(index, 0) + 1
    swinging = False
    swing_time = 0
    accepted: list[int] = []
    # Every tick runs, punch or not: the swing timer advances in the idle
    # ticks too (that is what ends a swing when the player stops clicking).
    for index in range(min(ticks), max(ticks) + 1):
        count = 0
        for _ in range(ticks.get(index, 0)):
            if not swinging or swing_time >= threshold or swing_time < 0:
                swinging = True
                swing_time = -1
                count += 1
        if count:
            accepted.append(count)
        # End of the tick: the timer runs after the packets.
        if swinging:
            swing_time += 1
            if swing_time >= DURATION:
                swinging = False
                swing_time = 0
    return accepted


def main() -> None:
    an = anchors(FOLDER)
    offset = an[CONN_B] - an[CONN_A]

    a = rows(FOLDER, CONN_A)
    b = rows(FOLDER, CONN_B)
    # Times are relative to the FIRST PACKET of A's connection (its zero of
    # `read_ms`), so the phase grid aligns with the connection's tick lattice;
    # this is the normalization the confirmed model uses.
    a0 = a[0]["read_ms"]
    punch_ms = [
        r["read_ms"] - a0
        for r in a
        if r.get("name") == "minecraft:punch" and r.get("dir") == "serverbound"
    ]
    swing_ms = sorted(
        r["read_ms"] + offset - a0
        for r in b
        if r.get("name") == "minecraft:swing_animation" and r.get("dir") == "clientbound"
    )
    print(f"punch packets: {len(punch_ms)}, observed swings: {len(swing_ms)}")
    observed = swing_clusters(swing_ms)
    print(f"observed clusters: {observed}")

    phases = int(TICK_MS / PHASE_STEP_MS)
    total = phases * len(DELAYS_MS) * len(THRESHOLDS)
    matched = 0
    for threshold in THRESHOLDS:
        hits = []
        for p in range(phases):
            phase = p * PHASE_STEP_MS
            for delay in DELAYS_MS:
                if model_clusters(punch_ms, phase, delay, threshold) == observed:
                    hits.append((phase, delay))
        matched += len(hits)
        print(f"threshold {threshold}: {len(hits)}/{phases * len(DELAYS_MS)} combos match")
        if hits and threshold == 3:
            sample = hits[0]
            print(f"  first match: phase {sample[0]} ms, delay {sample[1]} ms")

    print(f"total: {matched}/{total} combos reproduce the observed structure")
    # The confirmed test point (also used by the Rust unit test):
    print(
        "phase 39.5 ms, delay 0, threshold 3:",
        model_clusters(punch_ms, 39.5, 0.0, 3),
    )
    # The cluster-start intervals of B, for the report (the model gives
    # 200/200 at threshold 3 and 250/250 at threshold 4; the difference is
    # not explained by the trace).
    starts = []
    previous = None
    for t in swing_ms:
        if previous is None or t - previous >= CLUSTER_GAP_MS:
            starts.append(t)
        previous = t
    print("observed cluster starts (ms):", [round(s, 3) for s in starts])
    print("cluster-start gaps (ms):", [round(b - a, 3) for a, b in zip(starts, starts[1:])])


if __name__ == "__main__":
    main()
