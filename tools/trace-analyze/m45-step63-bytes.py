"""Check: do our body yaw bytes (move_entity_rot/pos_rot) match vanilla's body
bytes, with only the head byte differing? Walks both streams in emission order
and compares body-byte multisets and head-byte multisets."""
import json
import base64
from pathlib import Path
from collections import Counter

T = Path(r"E:\MineRust-data\traces")
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


def body_bytes(lines, bid):
    out = []
    for r in lines:
        if r.get("dir") != "clientbound":
            continue
        n = r.get("name")
        if n in ("minecraft:move_entity_pos_rot", "minecraft:move_entity_rot"):
            p = r["payload"]
            try:
                if varint(p)[0] != bid:
                    continue
            except Exception:
                continue
            out.append(p[-2])  # yaw byte (pitch is last)
    return out


def head_bytes(lines, bid):
    out = []
    for r in lines:
        if r.get("dir") != "clientbound":
            continue
        if r.get("name") == "minecraft:rotate_head":
            p = r["payload"]
            try:
                if varint(p)[0] != bid:
                    continue
            except Exception:
                continue
            out.append(p[-1])
    return out


for code in ("c2", "c4"):
    va = rows(T / f"two-{code}-A.jsonl")
    oa = rows(T / f"two-{code}-ours-A.jsonl")
    vid, oid = b_id(va), b_id(oa)
    vb, ob = body_bytes(va, vid), body_bytes(oa, oid)
    vh, oh = head_bytes(va, vid), head_bytes(oa, oid)
    print(f"===== {code} =====")
    print(f"  body bytes: vanilla {len(vb)} ours {len(ob)}; equal-as-multiset: {Counter(vb) == Counter(ob)}")
    print(f"    vanilla body: {Counter(vb)}")
    print(f"    ours   body: {Counter(ob)}")
    print(f"  head bytes: vanilla {len(vh)} ours {len(oh)}")
    print(f"    vanilla head: {Counter(vh)}")
    print(f"    ours   head: {Counter(oh)}")
    print(f"  vanilla body==head multiset? {Counter(vb) == Counter(vh)}")
    # our head == our body? vanilla head == vanilla body?
    print(f"  ours body vs ours head equal multiset: {Counter(ob) == Counter(oh)}")
    print(f"  our body == vanilla head multiset: {Counter(ob) == Counter(vh)}")
    print(f"  our head == vanilla body multiset: {Counter(oh) == Counter(vb)}")
    print()
