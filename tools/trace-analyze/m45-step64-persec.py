"""Шаг 6.4, part 1: packets about a moving player per observer per tick.
Vanilla C2/C4 vs ours, from the two-bot traces. The observation window is B's
movement phase (walking in C2, turning in C4), start to B-script end; ticks are
counted from B's serverbound tick counter (client_tick_end) — our own clock for
the window, no cross-trace time comparison."""
import json
import base64
from pathlib import Path

T = Path(r"E:\MineRust-data\traces")
B_UUID = "b3b03a00-4714-337e-8103-52c60a6c836f"
MOVES = {
    "minecraft:move_entity_pos", "minecraft:move_entity_pos_rot", "minecraft:move_entity_rot",
    "minecraft:rotate_head", "minecraft:entity_position_sync", "minecraft:set_entity_data",
    "minecraft:set_entity_motion", "minecraft:update_attributes", "minecraft:set_equipment",
    "minecraft:teleport_entity", "minecraft:set_passengers", "minecraft:entity_event",
}


def varint(b, p=0):
    v = s = 0
    while True:
        x = b[p]
        p += 1
        v |= (x & 0x7F) << s
        if not x & 0x80:
            break
        s += 7
    return v, p


def rows(p):
    out = []
    for line in open(p, encoding="utf-8", errors="replace"):
        line = line.strip()
        if not line:
            continue
        r = json.loads(line)
        r["payload"] = base64.b64decode(r.get("b64") or "")
        out.append(r)
    return out


def b_id(lines):
    for r in lines:
        if r.get("name") == "minecraft:add_entity":
            b = r["payload"]
            try:
                i, p = varint(b)
            except Exception:
                continue
            if b[p:p + 16].hex() == B_UUID.replace("-", ""):
                return i


def about(lines, bid):
    out = []
    for r in lines:
        if r.get("dir") != "clientbound":
            continue
        n = r.get("name")
        if n in ("minecraft:player_info_update", "minecraft:player_info_remove"):
            if B_UUID.replace("-", "") in r["payload"].hex():
                out.append(r)
            continue
        if n == "minecraft:remove_entities":
            b = r["payload"]
            cnt, p = varint(b)
            ids = []
            for _ in range(cnt):
                v, p = varint(b, p)
                ids.append(v)
            if bid in ids:
                out.append(r)
            continue
        if n in MOVES:
            try:
                v = varint(r["payload"])[0]
            except Exception:
                continue
            if v == bid:
                out.append(r)
    return out


for code in ("c2", "c4"):
    for side in ("", "-ours"):
        bl = rows(T / f"two-{code}{side}-B.jsonl")
        al = rows(T / f"two-{code}{side}-A.jsonl")
        bid = b_id(al)
        # B's tick counter: count of client_tick_end; window = ticks where B
        # sends a movement packet (the moving phase) -> t of first/last such
        move_ts = [r["t_ms"] for r in bl
                   if r.get("dir") == "serverbound"
                   and r.get("name") in ("minecraft:move_player_pos",
                                         "minecraft:move_player_pos_rot",
                                         "minecraft:move_player_rot")]
        tick_ends = [r["t_ms"] for r in bl if r.get("name") == "minecraft:client_tick_end"]
        ab = about(al, bid)
        # window on B's clock: first..last movement packet
        t0, t1 = move_ts[0], move_ts[-1]
        ticks = sum(1 for t in tick_ends if t0 <= t <= t1)
        pkts = [r for r in ab if t0 <= r["t_ms"] <= t1]
        per_tick = len(pkts) / ticks if ticks else 0.0
        kinds = {}
        for r in pkts:
            kinds[r.get("name")] = kinds.get(r.get("name"), 0) + 1
        label = f"{code}{'-ours' if side else ' vanilla'}"
        print(f"===== {label}: window {t0:.0f}..{t1:.0f} ms, {ticks} B-ticks =====")
        print(f"  packets about B in window: {len(pkts)}  -> {per_tick:.2f} per observer per tick")
        for k, v in sorted(kinds.items(), key=lambda x: -x[1]):
            print(f"    {v:4d} {k}")
        print()
