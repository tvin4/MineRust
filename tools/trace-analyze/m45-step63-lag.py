"""Test: vanilla body byte = client yaw lagged by k ticks, head byte = current?
B sends move_player_rot with (yaw, pitch) floats each tick; emitted bytes are floor(deg*256/360)."""
import json
import base64
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


def byte_of(deg):
    return int(deg * 256.0 / 360.0) % 256  # floor then mod 256


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


for code in ("c2", "c4"):
    b = rows(T / f"two-{code}-B.jsonl")
    a = rows(T / f"two-{code}-A.jsonl")
    bid = b_id(a)
    # B's yaw timeline from serverbound moves carrying yaw (pos_rot/rot):
    # move_player_rot: f32 yaw, f32 pitch, u8 flags (9 bytes)
    # move_player_pos_rot: 3xf64 pos, f32 yaw, f32 pitch, u8 flags
    yaws = []  # (t_ms, yaw_deg)
    for r in b:
        if r.get("dir") != "serverbound":
            continue
        n = r.get("name")
        p = r["payload"]
        try:
            if n == "minecraft:move_player_rot" and len(p) >= 8:
                yaw = struct.unpack(">f", p[0:4])[0]
            elif n == "minecraft:move_player_pos_rot" and len(p) >= 32:
                yaw = struct.unpack(">f", p[24:28])[0]
            else:
                continue
        except Exception:
            continue
        yaws.append((r["t_ms"], yaw))
    # emitted body/head bytes about B, in order
    ev = []
    for r in a:
        if r.get("dir") != "clientbound":
            continue
        n = r.get("name")
        p = r["payload"]
        if n in ("minecraft:move_entity_pos_rot", "minecraft:move_entity_rot", "minecraft:rotate_head"):
            try:
                if varint(p)[0] != bid:
                    continue
            except Exception:
                continue
            if n == "minecraft:rotate_head":
                ev.append((r["t_ms"], "head", p[-1]))
            elif n == "minecraft:move_entity_pos_rot":
                ev.append((r["t_ms"], "body", p[-2]))
            else:
                ev.append((r["t_ms"], "body", p[-2]))
    # for each emitted byte, find the client yaw sample whose byte matches, and
    # the nearest-in-time sample; report best lag in samples
    print(f"===== {code}: {len(yaws)} yaw samples, {len(ev)} emitted body/head bytes =====")
    for kind in ("head", "body"):
        lags = {}
        for t, k, byte in ev:
            if k != kind:
                continue
            # candidate samples: those whose byte matches; pick nearest in time
            best = None
            for i, (st, yaw) in enumerate(yaws):
                if byte_of(yaw) == byte:
                    d = abs(st - t)
                    if best is None or d < best[1]:
                        best = (i, d)
            if best:
                # lag relative to the nearest-in-time sample overall
                near = min(range(len(yaws)), key=lambda i: abs(yaws[i][0] - t))
                lags[best[0] - near] = lags.get(best[0] - near, 0) + 1
        print(f"  {kind}: matching-sample lag vs nearest sample: {dict(sorted(lags.items()))}")
    print()
