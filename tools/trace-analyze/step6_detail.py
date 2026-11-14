"""Detailed per-packet / per-tick dump for M4.5 Step 6.1.

Outputs (to stdout):
  * C1: every A packet about B, with t and type;
  * C2: B's serverbound move per tick (kind, x, yaw, pitch) and A's
    movement packets about B decoded (type, dx/dy/dz, yaw, pitch);
  * C4: same for the rotation scene.
"""

import base64
import json
import math
import struct
from pathlib import Path

TRACES = Path(r"E:\MineRust-data\traces")
B_UUID = bytes.fromhex("b3b03a004714337e810352c60a6c836f")

MOVE = "minecraft:move_entity_pos"
MOVE_ROT = "minecraft:move_entity_pos_rot"
ROT = "minecraft:move_entity_rot"
HEAD = "minecraft:rotate_head"
SYNC = "minecraft:entity_position_sync"
MOVEMENT = {MOVE, MOVE_ROT, ROT, HEAD, SYNC}


def rows(path):
    out = []
    with open(path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            row["payload"] = base64.b64decode(row.get("b64") or "")
            out.append(row)
    return out


def varint(buf, pos=0):
    value = 0
    shift = 0
    while True:
        byte = buf[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            break
        shift += 7
    if value & (1 << 31):
        value -= 1 << 32
    return value, pos


def decode_add(body):
    eid, pos = varint(body, 0)
    return eid, body[pos:pos + 16]


def bid_in_a(a_rows):
    for row in a_rows:
        if row.get("dir") == "clientbound" and row.get("name") == "minecraft:add_entity":
            eid, raw = decode_add(row["payload"])
            if raw == B_UUID:
                return eid
    return None


def remove_ids(body):
    count, pos = varint(body, 0)
    out = []
    for _ in range(count):
        value, pos = varint(body, pos)
        out.append(value)
    return out


def entity_id_of(name, body):
    if name == "minecraft:add_entity":
        return decode_add(body)[0]
    try:
        return varint(body, 0)[0]
    except (IndexError, struct.error):
        return None


def about(a_rows, bid):
    out = []
    for row in a_rows:
        if row.get("dir") != "clientbound":
            continue
        name = row.get("name")
        if name == "minecraft:remove_entities":
            if bid in remove_ids(row["payload"]):
                out.append(row)
        elif name == "minecraft:player_info_remove":
            out.append(row)
        elif entity_id_of(name, row["payload"]) == bid:
            out.append(row)
    return out


def decode_motion(row):
    name, body = row["name"], row["payload"]
    if name == MOVE:
        eid, pos = varint(body, 0)
        flags = body[pos]
        tail = len(body) - pos - 1
        if tail == 7:
            kind = body[pos + 1]
            off = pos + 2
        else:
            kind = None
            off = pos + 1
        dx, dy, dz = struct.unpack_from(">hhh", body, off)
        return f"{name.replace('minecraft:', '')} flags={flags} kind={kind} dx={dx} dy={dy} dz={dz}"
    if name == MOVE_ROT:
        eid, pos = varint(body, 0)
        flags = body[pos]
        tail = len(body) - pos - 1
        if tail == 9:
            kind = body[pos + 1]
            off = pos + 2
        else:
            kind = None
            off = pos + 1
        dx, dy, dz = struct.unpack_from(">hhh", body, off)
        yaw = body[off + 6]
        pitch = struct.unpack_from(">b", body, off + 7)[0]
        return f"{name.replace('minecraft:', '')} flags={flags} kind={kind} dx={dx} dy={dy} dz={dz} yaw=0x{yaw:02x} pitch={pitch}"
    if name == ROT:
        eid, pos = varint(body, 0)
        flags = body[pos]
        yaw = body[pos + 1]
        pitch = struct.unpack_from(">b", body, pos + 2)[0]
        return f"{name.replace('minecraft:', '')} flags={flags} yaw=0x{yaw:02x} pitch={pitch}"
    if name == HEAD:
        eid, pos = varint(body, 0)
        return f"{name.replace('minecraft:', '')} head=0x{body[pos]:02x}"
    if name == SYNC:
        eid, pos = varint(body, 0)
        teleport, pos = varint(body, pos)
        flags = body[pos]
        x, y, z = struct.unpack_from(">ddd", body, pos + 1)
        rotation = body[pos + 25]
        yaw = struct.unpack_from(">b", body, pos + 26)[0]
        pitch = struct.unpack_from(">b", body, pos + 27)[0]
        return (f"entity_position_sync teleport={teleport} flags={flags} "
                f"pos=({x:.2f},{y:.2f},{z:.2f}) rotation={rotation} yaw={yaw} pitch={pitch}")
    return name


def b_moves(b_rows):
    out = []
    tick = 0
    for row in b_rows:
        if row.get("dir") != "serverbound":
            continue
        name = row.get("name")
        if name not in ("minecraft:move_player_pos",
                        "minecraft:move_player_pos_rot",
                        "minecraft:move_player_rot"):
            continue
        tick += 1
        body = row["payload"]
        if name == "minecraft:move_player_pos":
            x, y, z = struct.unpack_from(">ddd", body, 0)
            yaw = pitch = None
            kind = "pos"
        elif name == "minecraft:move_player_pos_rot":
            x, y, z = struct.unpack_from(">ddd", body, 0)
            yaw, pitch = struct.unpack_from(">ff", body, 24)
            kind = "pos_rot"
        else:
            yaw, pitch = struct.unpack_from(">ff", body, 0)
            x = y = z = None
            kind = "rot"
        out.append({"tick": tick, "t": row["t_ms"], "kind": kind,
                    "x": x, "y": y, "z": z, "yaw": yaw, "pitch": pitch})
    return out


def byte_round(deg):
    return int(math.floor(deg * 256.0 / 360.0 + 0.5)) & 0xFF


def main():
    for code in ("c1", "c2", "c4"):
        a = rows(TRACES / f"two-{code}-A.jsonl")
        bid = bid_in_a(a)
        seq = about(a, bid)
        print(f"\n===== {code} A about B (id {bid}): {len(seq)} packets =====")
        if code == "c1":
            for i, row in enumerate(seq):
                print(f"  #{i:3d} t={row['t_ms']:9.1f} {row['name']}")
            continue

        b = rows(TRACES / f"two-{code}-B.jsonl")
        bm = b_moves(b)
        print(f"B moves: {len(bm)}")
        print("-- B moves (first 12, then every 10th) --")
        for m in bm[:12]:
            yaw = f"{m['yaw']:.2f}" if m['yaw'] is not None else "-"
            pitch = f"{m['pitch']:.2f}" if m['pitch'] is not None else "-"
            print(f"  tick={m['tick']:3d} t={m['t']:9.1f} {m['kind']:7} x={m['x']} yaw={yaw} pitch={pitch}")
        print("-- A movement packets about B --")
        for i, row in enumerate(seq):
            if row["name"] in MOVEMENT:
                print(f"  #{i:3d} t={row['t_ms']:9.1f} {decode_motion(row)}")


if __name__ == "__main__":
    main()