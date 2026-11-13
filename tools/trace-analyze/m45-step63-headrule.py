"""Hypothesis tests for vanilla rotate_head vs body yaw byte:
H1: head[i] == body[i-1] (head one broadcast behind)
H2: head[i] == body[i] (ours)
H3: head[i] == body[i] - one client tick step (9 deg = 6.4 bytes)
Run on vanilla C2/C4 body/head byte sequences in emission order."""
import json
import base64
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


def events(lines, bid):
    """(kind, byte) in emission order for body/head packets (sync = body too)."""
    out = []
    for r in lines:
        if r.get("dir") != "clientbound":
            continue
        n = r.get("name")
        p = r["payload"]
        try:
            if varint(p)[0] != bid:
                continue
        except Exception:
            continue
        if n in ("minecraft:move_entity_pos_rot", "minecraft:move_entity_rot"):
            out.append(("body", p[-2]))
        elif n == "minecraft:rotate_head":
            out.append(("head", p[-1]))
        elif n == "minecraft:entity_position_sync":
            # yaw byte at fixed offset of the 37-byte layout (tail4: yaw pitch ? ?)
            out.append(("sync", p[-4]))
    return out


for code in ("c2", "c4"):
    va = rows(T / f"two-{code}-A.jsonl")
    vid = b_id(va)
    ev = events(va, vid)
    bodies = [b for k, b in ev if k in ("body", "sync")]
    heads = [h for k, h in ev if k == "head"]
    # pair each head with the nearest preceding body index
    seq = ev
    body_at = [i for i, (k, _) in enumerate(seq) if k in ("body", "sync")]
    head_at = [i for i, (k, _) in enumerate(seq) if k == "head"]
    h1 = h2 = h3 = neither = 0
    examples = []
    for hi in head_at:
        # previous body index (in emission order)
        prev = [i for i in body_at if i < hi]
        cur = seq[hi][1]
        prev_b = seq[prev[-1]][1] if prev else None
        next_body = [i for i in body_at if i > hi]
        nxt_b = seq[next_body[0]][1] if next_body else None
        ok1 = prev_b == cur
        ok2 = nxt_b == cur
        ok3 = prev_b is not None and (prev_b - cur) % 256 in (6, 7)
        if ok1:
            h1 += 1
        elif ok2:
            h2 += 1
        elif ok3:
            h3 += 1
        else:
            neither += 1
            if len(examples) < 6:
                examples.append((hi, cur, prev_b, nxt_b))
    print(f"===== {code} vanilla: heads={len(head_at)} bodies(incl sync)={len(body_at)} =====")
    print(f"  H1 head==prev body:        {h1}")
    print(f"  H2 head==next body (ours): {h2}")
    print(f"  H3 head==prev body -6/7:   {h3}")
    print(f"  none of the three:         {neither}")
    for hi, cur, pb, nb in examples:
        print(f"    #{hi} head=0x{cur:02x} prev_body={pb and hex(pb)} next_body={nb and hex(nb)}")
    print()
