"""Шаг 7.1: dump player_info_update and set_entity_data for B from our traces
and the vanilla C1 trace: hex + field layout. Also which metadata index carries
the game-mode-dependent bits (entity flags index 0)."""
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
    """player_info_update layout: actions u8, then per entry uuid(16), name
    (varint str), then flags-dependent fields."""
    p = 0
    actions = b[p]
    p += 1
    uuid = b[p:p + 16].hex()
    p += 16
    nlen, p = varint(b, p)
    name = b[p:p + nlen].decode()
    p += nlen
    rest = b[p:]
    return actions, uuid, name, rest


for tag, fname in (("C1 vanilla", "two-c1-A.jsonl"),
                   ("C1 ours", "two-c1-ours-A.jsonl")):
    lines = rows(T / fname)
    bid = b_id(lines)
    print(f"===== {tag} (B entity id {bid}) =====")
    n_info = 0
    n_meta = 0
    for r in lines:
        if r.get("dir") != "clientbound":
            continue
        n = r.get("name")
        b = r["payload"]
        if n == "minecraft:player_info_update" and B_UUID.replace("-", "") in b.hex():
            n_info += 1
            try:
                actions, uuid, name, rest = dump_info(b)
            except Exception as e:
                print(f"  pi_up #{n_info} parse error {e}: {b.hex()}")
                continue
            print(f"  pi_up #{n_info} len={len(b)} hex={b.hex()}")
            print(f"    actions=0x{actions:02x} uuid={uuid} name={name!r}")
            print(f"    rest(hex)={rest.hex()}  (game_mode varint at start of rest per actions)")
            try:
                gm, p = varint(rest)
                print(f"    game_mode={gm} then={rest[p:].hex()}")
            except Exception:
                pass
        elif n == "minecraft:set_entity_data" and varint(b)[0] == bid:
            n_meta += 1
            eid, p = varint(b)
            print(f"  meta #{n_meta} len={len(b)} hex={b.hex()}  (id={eid}, meta={b[p:].hex()})")
    print(f"  totals: pi_up={n_info} meta={n_meta}")
    print()
