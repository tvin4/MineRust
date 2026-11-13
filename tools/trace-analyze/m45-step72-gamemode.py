"""M4.5 step 7.2: player_info_update and set_entity_data for B in the
non-spectator vanilla runs (C5 survival, C6 creative) against the
spectator C1 run: hex, field layout and byte diff.

Usage: python m45-step72-gamemode.py > target/bench/step72-gamemode.txt
"""
import base64
import json
from pathlib import Path

T = Path(r"C:\MineRust-data\traces")
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
    p += 1
    cnt, p = varint(b, p)
    print(f"  [0] actions=0x{actions:02x}  [{p - 1}] count={cnt}")
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
            extra = f" (0x{val[0]:02x}={val[0]})"
        elif typ == 3:
            val = b[q:q + 4]
            q += 4
            extra = ""
        else:
            print(f"  [{p}] index={idx} type={typ} UNKNOWN TYPE SIZE")
            return
        print(f"  [{p}] index={idx} type={typ} [{start}..{q}) value={val.hex()}{extra}")
        p = q
    assert p == len(b), (p, len(b))


def collect(fname):
    """(pi_up payloads in order, meta payloads in order) about B."""
    lines = rows(T / fname)
    bid = b_id(lines)
    infos, metas = [], []
    for r in lines:
        if r.get("dir") != "clientbound":
            continue
        n = r.get("name")
        b = r["payload"]
        if n == "minecraft:player_info_update" and B_UUID.replace("-", "") in b.hex():
            infos.append(b)
        elif n == "minecraft:set_entity_data" and varint(b)[0] == bid:
            metas.append(b)
    return bid, infos, metas


RUNS = [
    ("C1 vanilla (spectator)", "two-c1-A.jsonl"),
    ("C5 vanilla (survival)", "two-c5-A.jsonl"),
    ("C6 vanilla (creative)", "two-c6-A.jsonl"),
]

all_bytes = {}
for tag, fname in RUNS:
    bid, infos, metas = collect(fname)
    print(f"===== {tag} (B entity id {bid}) =====")
    for k, b in enumerate(infos, 1):
        print(f"player_info_update #{k} len={len(b)}")
        print(f"  hex={b.hex()}")
        dump_info(b)
    for k, b in enumerate(metas, 1):
        print(f"set_entity_data #{k} len={len(b)}")
        print(f"  hex={b.hex()}")
        dump_meta(b)
    print(f"  totals: pi_up={len(infos)} meta={len(metas)}")
    print()
    all_bytes[tag] = (infos, metas)

print("===== byte diff against C1 (spectator) =====")
base_tag = "C1 vanilla (spectator)"
base = all_bytes[base_tag]
for tag, (infos, metas) in all_bytes.items():
    if tag == base_tag:
        continue
    for kind, theirs, mine in (("pi_up", infos, base[0]), ("meta", metas, base[1])):
        for k in range(max(len(theirs), len(mine))):
            a = theirs[k] if k < len(theirs) else None
            b = mine[k] if k < len(mine) else None
            if a == b:
                print(f"{tag} {kind} #{k + 1}: identical")
                continue
            if a is None or b is None:
                print(f"{tag} {kind} #{k + 1}: count differs "
                      f"({'absent' if a is None else len(a)} vs {'absent' if b is None else len(b)})")
                continue
            diffs = [(i, x, y) for i, (x, y) in enumerate(zip(a, b)) if x != y]
            if len(a) != len(b):
                print(f"{tag} {kind} #{k + 1}: len {len(a)} vs {len(b)}")
            for i, x, y in diffs:
                print(f"{tag} {kind} #{k + 1}: offset {i}: {x:02x} vs {y:02x}")
