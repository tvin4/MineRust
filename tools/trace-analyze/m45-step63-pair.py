"""Vanilla C2/C4: pair each rotate_head to the nearest body packet by yaw byte."""
import json
import base64
from pathlib import Path

T = Path(r"C:\MineRust-data\traces")
B_UUID = "b3b03a00-4714-337e-8103-52c60a6c836f"
MOVES = {
    "minecraft:move_entity_pos", "minecraft:move_entity_pos_rot", "minecraft:move_entity_rot",
    "minecraft:rotate_head", "minecraft:entity_position_sync", "minecraft:set_entity_data",
    "minecraft:update_attributes",
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
        if n in MOVES or n == "minecraft:entity_position_sync":
            try:
                v = varint(r["payload"])[0]
            except Exception:
                continue
            if v == bid:
                out.append(r)
    return out


def short(n):
    return {
        "minecraft:move_entity_pos": "pos",
        "minecraft:move_entity_pos_rot": "pos_rot",
        "minecraft:move_entity_rot": "rot",
        "minecraft:rotate_head": "head",
        "minecraft:entity_position_sync": "sync",
        "minecraft:set_entity_data": "meta",
        "minecraft:update_attributes": "attrs",
        "minecraft:remove_entities": "rm",
        "minecraft:player_info_remove": "pi_rm",
        "minecraft:player_info_update": "pi_up",
    }[n]


for code in ("c2", "c4"):
    for side in ("", "-ours"):
        lines = rows(T / f"two-{code}{side}-A.jsonl")
        bid = b_id(lines)
        ab = about(lines, bid)
        # body yaw bytes by index
        body = {}   # idx -> yaw byte
        sync = {}   # idx -> (yaw-ish tail?)
        head = {}   # idx -> yaw byte
        for i, r in enumerate(ab):
            n = r.get("name")
            b = r["payload"]
            if n in ("minecraft:move_entity_pos_rot", "minecraft:move_entity_rot"):
                body[i] = b[-2] if n == "minecraft:move_entity_pos_rot" else b[-2]
            elif n == "minecraft:move_entity_rot":
                body[i] = b[-2]
            elif n == "minecraft:rotate_head":
                head[i] = b[-1]
            elif n == "minecraft:entity_position_sync":
                sync[i] = b
        print(f"===== {code}{side} n={len(ab)} bodies={len(body)} heads={len(head)} sync={len(sync)} =====")
        # for each head, find the nearest body with same yaw
        unmatched = 0
        offsets = []
        body_list = sorted(body.items())
        for hi, hy in sorted(head.items()):
            best = None
            for bi, by in body_list:
                if by == hy:
                    d = abs(bi - hi)
                    if best is None or d < best[1]:
                        best = (bi, d)
            if best is None:
                unmatched += 1
            else:
                offsets.append(best[1])
        from collections import Counter
        print(f"  head->body |offset| dist: {Counter(offsets)}")
        print(f"  heads with no same-yaw body: {unmatched}")
        # show seq window around a mismatched head
        if side == "":
            for i in list(range(0, 20)):
                r = ab[i]
                b = r["payload"]
                print(f"    {i:3d} {r['t_ms']:9.1f} {short(r.get('name')):7s} {b.hex()}")
        print()
