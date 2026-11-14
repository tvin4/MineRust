"""Classify the (в) residuals of 6.3: head/body pairing by yaw, sync/zero-delta inventory."""
import json
import base64
from pathlib import Path
from collections import Counter

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
    return None


def about(lines, bid):
    out = []
    for r in lines:
        if r.get("dir") != "clientbound":
            continue
        n = r.get("name")
        if n == "minecraft:player_info_update":
            if B_UUID.replace("-", "") in r["payload"].hex():
                out.append(r)
            continue
        if n == "minecraft:player_info_remove":
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
                v = varint(r["payload"][1:])[0] if n == "minecraft:entity_event" else varint(r["payload"])[0]
            except Exception:
                continue
            if v == bid:
                out.append(r)
    return out


def body_yaw(bytes_):
    """Body yaw byte of move_entity_pos_rot/rot (last-but-one byte = yaw, pitch last)."""
    return bytes_[-2] if len(bytes_) >= 2 else None


def head_yaw(bytes_):
    return bytes_[-1]


for code in ("c2", "c4"):
    for side in ("", "-ours"):
        lines = rows(T / f"two-{code}{side}-A.jsonl")
        bid = b_id(lines)
        ab = about(lines, bid)
        kinds = Counter()
        # pairing: consecutive body-packet followed by head with equal yaw?
        pair_same = 0
        pair_diff = 0
        heads = []
        bodies = []
        for i, r in enumerate(ab):
            n = r.get("name")
            if n in ("minecraft:move_entity_pos_rot", "minecraft:move_entity_rot"):
                bodies.append((i, body_yaw(r["payload"])))
            elif n == "minecraft:rotate_head":
                heads.append((i, head_yaw(r["payload"])))
            if n == "minecraft:move_entity_pos" and len(r["payload"]) == 9:
                kinds["pos9"] += 1
            elif n == "minecraft:move_entity_pos" and len(r["payload"]) == 8:
                kinds["pos8-spawn"] += 1
            elif n == "minecraft:entity_position_sync":
                kinds["sync"] += 1
        # how many heads immediately follow a body with the same yaw byte?
        for (bi, by), (hi, hy) in zip(bodies, heads):
            if hi == bi + 1:
                if by == hy:
                    pair_same += 1
                else:
                    pair_diff += 1
        print(f"===== {code}{side} n={len(ab)} =====")
        print(f"  pos9={kinds['pos9']} spawn-zero={kinds['pos8-spawn']} sync={kinds['sync']}")
        print(f"  heads={len(heads)} bodies={len(bodies)} pair_same_yaw={pair_same} pair_diff_yaw={pair_diff}")
        # first 3 head/body pairs
        for (bi, by), (hi, hy) in list(zip(bodies, heads))[:3]:
            print(f"    body#{bi} yaw=0x{by:02x} head#{hi} yaw=0x{hy:02x}")
        # sync payloads: rotation bytes at end
        syncs = [(i, r) for i, r in enumerate(ab) if r.get("name") == "minecraft:entity_position_sync"]
        for i, r in syncs:
            b = r["payload"]
            print(f"    sync#{i} len={len(b)} tail4={b[-4:].hex()}")
        print()
