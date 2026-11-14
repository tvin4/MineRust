"""M4.5 Step 6.1: map every A packet about B back to a B tick and derive
the send rule (vanilla and ours traces).

For each scenario and each side the script:

1. parses B's serverbound movement packets into a per-tick state series
   (position and angles carried forward, exactly what the server knows);
2. reconstructs the absolute state A was told about from the clientbound
   relative packets (the fixed-point delta codec, floor on the byte angles);
3. matches every A movement packet to the B state index it reproduces;
4. reports, per A event, the B tick gap, the changed fields and the packet
   type, and aggregates the rule with confirmation counts and exceptions.
"""

import base64
import json
import math
import struct
import sys
from collections import Counter
from pathlib import Path

TRACES = Path(r"E:\MineRust-data\traces")
B_UUID = bytes.fromhex("b3b03a004714337e810352c60a6c836f")

MOVE = "minecraft:move_entity_pos"
MOVE_ROT = "minecraft:move_entity_pos_rot"
ROT = "minecraft:move_entity_rot"
HEAD = "minecraft:rotate_head"
SYNC = "minecraft:entity_position_sync"
MOVEMENT = {MOVE, MOVE_ROT, ROT, HEAD, SYNC}
SERVERBOUND = {
    "minecraft:move_player_pos",
    "minecraft:move_player_pos_rot",
    "minecraft:move_player_rot",
}


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
    raw = body[pos:pos + 16]
    pos += 16
    _, pos = varint(body, pos)  # entity type
    x, y, z = struct.unpack_from(">ddd", body, pos)
    return eid, raw, (x, y, z)


def bid_in(a_rows):
    for row in a_rows:
        if row.get("dir") == "clientbound" and row.get("name") == "minecraft:add_entity":
            eid, raw, pos = decode_add(row["payload"])
            if raw == B_UUID:
                return eid, pos
    return None, None


def remove_ids(body):
    count, pos = varint(body, 0)
    out = []
    for _ in range(count):
        value, pos = varint(body, pos)
        out.append(value)
    return out


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
        elif name and name.startswith("minecraft:"):
            try:
                if name == "minecraft:add_entity":
                    if decode_add(row["payload"])[0] == bid:
                        out.append(row)
                elif name != "minecraft:player_info_update" and varint(row["payload"], 0)[0] == bid:
                    out.append(row)
            except (IndexError, struct.error):
                pass
    return out


def b_states(b_rows, spawn_pos=None):
    """State after every serverbound movement packet: (tick, t, pos, yaw, pitch,
    kind). Position and angles carry forward."""
    pos = spawn_pos
    yaw = 0.0
    pitch = 0.0
    out = []
    tick = 0
    for row in b_rows:
        if row.get("dir") != "serverbound" or row.get("name") not in SERVERBOUND:
            continue
        tick += 1
        body = row["payload"]
        name = row["name"]
        if name == "minecraft:move_player_pos":
            pos = struct.unpack_from(">ddd", body, 0)
            kind = "pos"
        elif name == "minecraft:move_player_pos_rot":
            pos = struct.unpack_from(">ddd", body, 0)
            yaw, pitch = struct.unpack_from(">ff", body, 24)
            kind = "pos_rot"
        else:
            yaw, pitch = struct.unpack_from(">ff", body, 0)
            kind = "rot"
        out.append({"tick": tick, "t": row["t_ms"], "pos": pos,
                    "yaw": yaw, "pitch": pitch, "kind": kind})
    return out


def angle_floor(deg):
    return int(math.floor(deg * 256.0 / 360.0)) & 0xFF


def angle_round(deg):
    return int(math.floor(deg * 256.0 / 360.0 + 0.5)) & 0xFF


def signed(byte):
    return byte - 256 if byte >= 128 else byte


def decode_event(row):
    """Absolute/relative fields of one movement packet."""
    name, body = row["name"], row["payload"]
    if name == MOVE:
        eid, p = varint(body, 0)
        flags = body[p]
        tail = len(body) - p - 1
        kind = body[p + 1] if tail == 7 else None
        off = p + 2 if tail == 7 else p + 1
        dx, dy, dz = struct.unpack_from(">hhh", body, off)
        return {"flags": flags, "kind": kind, "d": (dx, dy, dz)}
    if name == MOVE_ROT:
        eid, p = varint(body, 0)
        flags = body[p]
        tail = len(body) - p - 1
        kind = body[p + 1] if tail == 9 else None
        off = p + 2 if tail == 9 else p + 1
        dx, dy, dz = struct.unpack_from(">hhh", body, off)
        yaw = body[off + 6]
        pitch = signed(body[off + 7])
        return {"flags": flags, "kind": kind, "d": (dx, dy, dz),
                "yaw": yaw, "pitch": pitch}
    if name == ROT:
        eid, p = varint(body, 0)
        flags = body[p]
        return {"flags": flags, "yaw": body[p + 1], "pitch": signed(body[p + 2])}
    if name == HEAD:
        eid, p = varint(body, 0)
        return {"head": body[p]}
    if name == SYNC:
        eid, p = varint(body, 0)
        teleport, p = varint(body, p)
        flags = body[p]
        x, y, z = struct.unpack_from(">ddd", body, p + 1)
        rotation = body[p + 25]
        yaw, pitch = struct.unpack_from(">ff", body, p + 26)
        tail = body[p + 34] if len(body) > p + 34 else None
        return {"teleport": teleport, "flags": flags, "pos": (x, y, z),
                "rotation": rotation, "yaw": yaw, "pitch": pitch, "tail": tail}
    return {}


def ev_angle_bytes(ev, name):
    """The yaw/pitch bytes a packet carries (sync carries f32 degrees)."""
    if name == SYNC:
        return angle_floor(ev["yaw"]), angle_floor(ev["pitch"])
    return ev["yaw"], ev["pitch"] & 0xFF


def match_state(events, states, spawn_pos):
    """Reconstruct the state after each movement event and match it to the
    nearest B state: (position 1/4096 tolerance, byte-equal angles)."""
    cur = list(spawn_pos)
    yaw = 0.0
    pitch = 0.0
    anchors = []
    for row in events:
        if row["name"] == HEAD:
            continue
        ev = decode_event(row)
        if row["name"] in (MOVE, MOVE_ROT):
            dx, dy, dz = ev["d"]
            cur[0] += dx / 4096.0
            cur[1] += dy / 4096.0
            cur[2] += dz / 4096.0
        if row["name"] == SYNC:
            cur = list(ev["pos"])
        if "yaw" in ev and row["name"] != HEAD:
            yaw = signed(ev["yaw"]) * 360.0 / 256.0
            pitch = ev["pitch"] * 360.0 / 256.0
        if row["name"] == SYNC:
            yaw, pitch = ev["yaw"], ev["pitch"]
        # Match to the closest B state by position, then by angle bytes.
        yb = angle_floor(yaw)
        pb_ours = angle_floor(pitch)
        best = None
        best_key = None
        for st in states:
            if st["pos"] is None:
                continue
            dist = max(abs(st["pos"][i] - cur[i]) for i in range(3))
            same_angle = (angle_floor(st["yaw"]) == yb and
                          angle_floor(st["pitch"]) == pb_ours)
            key = (0 if same_angle else 1, dist)
            if best_key is None or key < best_key:
                best_key = key
                best = st
        anchors.append({"row": row, "ev": ev, "pos": tuple(cur),
                        "yaw": yaw, "pitch": pitch, "state": best,
                        "dist": best_key[1] if best_key else None,
                        "same_angle": best_key[0] == 0 if best_key else None})
    return anchors


def describe(label, path_a, path_b):
    a = rows(path_a)
    bid, spawn_pos = bid_in(a)
    if bid is None:
        print(f"{label}: B never spawned", file=sys.stdout)
        return
    seq = [r for r in about(a, bid) if r["name"] not in
           ("minecraft:remove_entities", "minecraft:player_info_remove",
            "minecraft:add_entity", "minecraft:player_info_update")]
    b = rows(path_b)
    states = b_states(b, spawn_pos)
    events = [r for r in seq if r["name"] in MOVEMENT]
    anchors = match_state(events, states, spawn_pos)
    print(f"\n===== {label}: id {bid}, A packets {len(seq)}, B moves {len(states)} =====",
          file=sys.stdout)
    print("types: " + str(Counter(r["name"].replace("minecraft:", "") for r in seq)),
          file=sys.stdout)
    print("head-counts: " + str(Counter(r["name"].replace("minecraft:", "")
                                        for r in seq if r["name"] in MOVEMENT)),
          file=sys.stdout)
    print("-- events: B tick, gap, fields changed, packet --", file=sys.stdout)
    prev = None
    gaps = Counter()
    rules = Counter()
    for i, a_ in enumerate(anchors):
        st = a_["state"]
        tick = st["tick"] if st else None
        gap = tick - prev if (tick is not None and prev is not None) else None
        if gap is not None:
            gaps[gap] += 1
        changed = []
        if prev is not None:
            p = next((s for s in states if s["tick"] == prev), None)
            if p is not None and st is not None and p["pos"] and st["pos"]:
                if any(abs(p["pos"][k] - st["pos"][k]) > 1e-4 for k in range(3)):
                    changed.append("pos")
            if p is not None and st is not None and (
                    angle_floor(p["yaw"]) != angle_floor(st["yaw"]) or
                    angle_floor(p["pitch"]) != angle_floor(st["pitch"])):
                changed.append("rot")
        kind = a_["row"]["name"].replace("minecraft:", "")
        rules[(kind, tuple(changed))] += 1
        if i < 40 or i > len(anchors) - 8:
            print(f"  #{i:3d} t={a_['row']['t_ms']:9.1f} tick={tick} gap={gap} "
                  f"changed={changed} {kind} dist={a_['dist'] if a_['dist'] is not None else -1:.4f} "
                  f"angle_ok={a_['same_angle']} state=({st['pos'] if st else None},"
                  f"{(st['yaw'] if st else 0):.1f},{(st['pitch'] if st else 0):.1f})",
                  file=sys.stdout)
        if tick is not None:
            prev = tick
    print("gap histogram: " + str(sorted(gaps.items())), file=sys.stdout)
    print("rule histogram (packet, changed):", file=sys.stdout)
    for key, count in rules.most_common():
        print(f"  {count:4d}  {key}", file=sys.stdout)

    # Never-sent B states: check whether every B state was matched.
    matched = {a_["state"]["tick"] for a_ in anchors if a_["state"]}
    unsent = [s["tick"] for s in states if s["tick"] not in matched]
    print(f"B states never reconstructed by an A packet: {len(unsent)}/{len(states)}",
          file=sys.stdout)

    # Angle-byte check for every rotation-bearing packet.
    bad_floor = 0
    bad_round = 0
    total = 0
    floor_mismatch_at = []
    for a_ in anchors:
        st = a_["state"]
        ev = a_["ev"]
        if st is None or "yaw" not in ev or a_["row"]["name"] == HEAD:
            continue
        total += 1
        yb, pb = ev_angle_bytes(ev, a_["row"]["name"])
        if angle_floor(st["yaw"]) != yb or angle_floor(st["pitch"]) != pb:
            bad_floor += 1
            floor_mismatch_at.append((st["tick"], a_["row"]["name"].replace("minecraft:", ""),
                                      yb, angle_floor(st["yaw"]),
                                      pb, angle_floor(st["pitch"])))
        if angle_round(st["yaw"]) != yb or angle_round(st["pitch"]) != pb:
            bad_round += 1
    print(f"angle byte: floor mismatches {bad_floor}/{total}, round mismatches "
          f"{bad_round}/{total}", file=sys.stdout)
    if floor_mismatch_at:
        print("  floor mismatches (tick, packet, our_yaw, state_yaw, our_pitch, "
              "state_pitch): " + str(floor_mismatch_at[:20]), file=sys.stdout)


def main():
    for code in ("c1", "c2", "c4"):
        for side in ("", "-ours"):
            a = TRACES / f"two-{code}{side}-A.jsonl"
            b = TRACES / f"two-{code}{side}-B.jsonl"
            if a.is_file() and b.is_file():
                describe(f"{code}{side or ' vanilla'}", a, b)


if __name__ == "__main__":
    main()