"""Шаг 7.1: layout parse with offsets of player_info_update and set_entity_data
for B (our C1 trace and vanilla C1 trace)."""
import base64
import json
import struct
from pathlib import Path

T = Path(r"E:\MineRust-data\traces")
B_UUID = "b3b03a00-4714-337e-8103-52c60a6c836f"


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


def dump_info(b):
    p = 0
    actions = b[p]
    print(f"  [0] actions=0x{actions:02x}")
    p += 1
    cnt, p = varint(b, p)
    print(f"  [{p - 1}] count={cnt}")
    for k in range(cnt):
        print(f"  [{p}..{p + 16}) uuid[{k}]={b[p:p + 16].hex()}")
        p += 16
    for k in range(cnt):
        nlen, p = varint(b, p)
        name = b[p:p + nlen].decode()
        print(f"  [{p - 1}] name_len={nlen} [{p}..{p + nlen}) name={name!r}")
        p += nlen
        pc, p = varint(b, p)
        print(f"  [{p - 1}] properties={pc}")
        print(f"  [{p}] init_chat=0x{b[p]:02x}")
        p += 1
        gm, p = varint(b, p)
        print(f"  [{p - 1}] game_mode={gm}")
        print(f"  [{p}] listed=0x{b[p]:02x}")
        p += 1
        lat, p = varint(b, p)
        print(f"  [{p - 1}] latency={lat}")
        print(f"  [{p}] display_name_absent=0x{b[p]:02x}")
        p += 1
        print(f"  [{p}] chat_session_absent=0x{b[p]:02x}")
        p += 1
        print(f"  [{p}] last_action=0x{b[p]:02x}")
        p += 1
    assert p == len(b), (p, len(b))


def dump_meta(b):
    eid, p = varint(b, 0)
    print(f"  [0] entity_id={eid}")
    while True:
        idx = b[p]
        if idx == 0xFF:
            print(f"  [{p}] terminator=0xff")
            p += 1
            break
        typ, q = varint(b, p + 1)
        start = q
        if typ == 0:
            val = b[q:q + 1]
            q += 1
        elif typ == 3:
            val = b[q:q + 4]
            q += 4
        else:
            print(f"  [{p}] index={idx} type={typ} UNKNOWN TYPE SIZE")
            return
        print(f"  [{p}] index={idx} type={typ} [{start}..{q}) value={val.hex()}"
              + (f" (f32={struct.unpack('>f', val)[0]})" if typ == 3 else f" (0x{val[0]:02x}={val[0]})"))
        p = q
    assert p == len(b), (p, len(b))


for tag, fname in (("C1 vanilla (B id 77)", "two-c1-A.jsonl"),
                   ("C1 ours (B id 2)", "two-c1-ours-A.jsonl")):
    lines = rows(T / fname)
    bid = b_id(lines)
    print(f"===== {tag} =====")
    n_info = n_meta = 0
    for r in lines:
        if r.get("dir") != "clientbound":
            continue
        n = r.get("name")
        b = r["payload"]
        if n == "minecraft:player_info_update" and B_UUID.replace("-", "") in b.hex():
            n_info += 1
            print(f"player_info_update #{n_info} len={len(b)}")
            print(f"  hex={b.hex()}")
            dump_info(b)
        elif n == "minecraft:set_entity_data" and varint(b)[0] == bid:
            n_meta += 1
            print(f"set_entity_data #{n_meta} len={len(b)}")
            print(f"  hex={b.hex()}")
            dump_meta(b)
    print()
