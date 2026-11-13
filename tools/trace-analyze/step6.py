"""M4.5 Step 6.1: derive the second-player packet rules from the vanilla traces.

Reads the vanilla two-bot traces (C1/C2/C4) and, for the ours traces when
present, reports:
  * B's serverbound movement timeline (kind, position, yaw, pitch) per tick;
  * A's clientbound packets about B, decoded;
  * cadence / change rules per packet type;
  * the angle-encoding check (byte_angle formula) for vanilla and ours;
  * the update_attributes record order;
  * the join player_info_update packets.

Layouts are the same length-pinned hypotheses as analyze.py.
"""

import base64
import json
import struct
import sys
from collections import Counter, defaultdict
from pathlib import Path

TRACES = Path(r"C:\MineRust-data\traces")

# Offline UUIDs of the bots (protocol::offline_uuid).
B_UUID = bytes.fromhex("b3b03a004714337e810352c60a6c836f")
A_UUID = bytes.fromhex("ac57753668593fbbbd68b9f062569092")

ENTITY_KEYS = {
    "minecraft:add_entity", "minecraft:remove_entities",
    "minecraft:move_entity_pos", "minecraft:move_entity_pos_rot",
    "minecraft:move_entity_rot", "minecraft:rotate_head",
    "minecraft:entity_position_sync", "minecraft:set_entity_data",
    "minecraft:set_entity_motion", "minecraft:update_attributes",
    "minecraft:teleport_entity", "minecraft:entity_event",
    "minecraft:set_equipment", "minecraft:set_passengers",
    "minecraft:player_info_update", "minecraft:player_info_remove",
    "minecraft:set_entity_data",
}

SERVERBOUND_MOVES = {
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
    etype, pos = varint(body, pos)
    x, y, z = struct.unpack_from(">ddd", body, pos)
    return eid, raw, etype, (x, y, z)


def decode_pos(body):
    return struct.unpack_from(">ddd", body, 0)


def decode_pos_rot(body):
    x, y, z = struct.unpack_from(">ddd", body, 0)
    yaw, pitch = struct.unpack_from(">ff", body, 24)
    return (x, y, z), yaw, pitch


def decode_rot(body):
    yaw, pitch = struct.unpack_from(">ff", body, 0)
    return yaw, pitch


def entity_id_of(row):
    name = row["name"]
    body = row["payload"]
    if name == "minecraft:add_entity":
        return decode_add(body)[0]
    try:
        if name == "minecraft:entity_event":
            return varint(body, 1)[0]
        if name in ENTITY_KEYS:
            return varint(body, 0)[0]
    except (IndexError, struct.error):
        return None
    return None


def remove_ids(body):
    count, pos = varint(body, 0)
    ids = []
    for _ in range(count):
        value, pos = varint(body, pos)
        ids.append(value)
    return ids


def b_entity_in_a(a_rows):
    for row in a_rows:
        if row.get("dir") != "clientbound" or row.get("name") != "minecraft:add_entity":
            continue
        eid, raw, _, _ = decode_add(row["payload"])
        if raw == B_UUID:
            return eid
    return None


def about_b(a_rows, bid):
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
        elif entity_id_of(row) == bid:
            out.append(row)
    return out


def b_moves(b_rows):
    out = []
    tick = 0
    for row in b_rows:
        if row.get("dir") != "serverbound" or row.get("name") not in SERVERBOUND_MOVES:
            continue
        tick += 1
        name = row["name"]
        body = row["payload"]
        if name == "minecraft:move_player_pos":
            pos = decode_pos(body)
            yaw = pitch = None
        elif name == "minecraft:move_player_pos_rot":
            pos, yaw, pitch = decode_pos_rot(body)
        else:
            yaw, pitch = decode_rot(body)
            pos = None
        out.append({"t": row["t_ms"], "seq": row["seq"], "name": name,
                    "pos": pos, "yaw": yaw, "pitch": pitch, "tick": tick})
    return out


def byte_angle_round(deg):
    # Python round is banker's; Java/Minecraft rounds half up.
    import math
    return int(math.floor(deg * 256.0 / 360.0 + 0.5)) & 0xFF


def byte_angle_floor(deg):
    import math
    return int(math.floor(deg * 256.0 / 360.0)) & 0xFF


def decode_packet(name, body):
    """Decoded interesting fields of one clientbound entity packet."""
    if name == "minecraft:move_entity_pos":
        eid, pos = varint(body, 0)
        flags = body[pos]
        # second header byte if the tail is 7 bytes, else 6
        tail = len(body) - pos - 1
        if tail == 7:
            kind = body[pos + 1]
            off = pos + 2
        else:
            kind = None
            off = pos + 1
        dx, dy, dz = struct.unpack_from(">hhh", body, off)
        return {"id": eid, "flags": flags, "kind": kind,
                "dx": dx, "dy": dy, "dz": dz}
    if name == "minecraft:move_entity_pos_rot":
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
        return {"id": eid, "flags": flags, "kind": kind,
                "dx": dx, "dy": dy, "dz": dz, "yaw": yaw, "pitch": pitch}
    if name == "minecraft:move_entity_rot":
        eid, pos = varint(body, 0)
        flags = body[pos]
        yaw = body[pos + 1]
        pitch = struct.unpack_from(">b", body, pos + 2)[0]
        return {"id": eid, "flags": flags, "yaw": yaw, "pitch": pitch}
    if name == "minecraft:rotate_head":
        eid, pos = varint(body, 0)
        return {"id": eid, "head_yaw": body[pos]}
    if name == "minecraft:entity_position_sync":
        eid, pos = varint(body, 0)
        teleport, pos = varint(body, pos)
        flags = body[pos]
        x, y, z = struct.unpack_from(">ddd", body, pos + 1)
        rotation = body[pos + 25]
        yaw = struct.unpack_from(">b", body, pos + 26)[0]
        pitch = struct.unpack_from(">b", body, pos + 27)[0]
        return {"id": eid, "teleport": teleport, "flags": flags,
                "pos": (x, y, z), "rotation": rotation, "yaw": yaw, "pitch": pitch}
    if name == "minecraft:update_attributes":
        eid, pos = varint(body, 0)
        count, pos = varint(body, pos)
        records = []
        for _ in range(count):
            key, pos = varint(body, pos)
            base = struct.unpack_from(">d", body, pos)[0]
            pos += 8
            mods, pos = varint(body, pos)
            records.append((key, base, mods))
        return {"id": eid, "records": records}
    return {}


def summarize(label, path_a, path_b, ours=False):
    print(f"\n########## {label} ##########", file=sys.stdout)
    a_rows = rows(path_a)
    bid = b_entity_in_a(a_rows)
    if bid is None:
        print("B never spawned in A's trace", file=sys.stdout)
        return
    about = about_b(a_rows, bid)
    print(f"A about B id {bid}: {len(about)} packets", file=sys.stdout)
    print("type counts: " + str(Counter(r['name'] for r in about)), file=sys.stdout)

    # Cadence: intervals of the response to a standing player.
    pos_packets = [r for r in about if r["name"] == "minecraft:move_entity_pos"]
    print(f"move_entity_pos: {len(pos_packets)}", file=sys.stdout)
    if len(pos_packets) > 1:
        times = [r["t_ms"] for r in pos_packets]
        gaps = [b - a for a, b in zip(times, times[1:])]
        print("  special gaps (len>2 bytes = real delta): " +
              str([round(g, 1) for g in gaps[:40]]), file=sys.stdout)

    if path_b is None:
        return
    b_rows = rows(path_b)
    bm = b_moves(b_rows)
    print(f"B serverbound moves: {len(bm)} " +
          str(Counter(m['name'] for m in bm)), file=sys.stdout)
    if not bm:
        return

    # Offset estimate: first B move vs first A movement packet about B that
    # carries a real (non-zero) delta or a rotation.
    first_a = None
    for row in about:
        name = row["name"]
        decoded = decode_packet(name, row["payload"])
        if name == "minecraft:move_entity_pos" and (decoded.get("dx"), decoded.get("dy"), decoded.get("dz")) != (0, 0, 0):
            first_a = row
            break
        if name in ("minecraft:move_entity_pos_rot", "minecraft:move_entity_rot"):
            first_a = row
            break
    if first_a is not None:
        offset = first_a["t_ms"] - bm[0]["t"]
        print(f"offset estimate (first move): {offset:.1f} ms", file=sys.stdout)
    else:
        offset = None

    # Per-tick table (compact: ranges + first/last 20).
    if offset is not None:
        # bucket A's about-B packets by nearest B move tick
        buckets = defaultdict(list)
        for row in about:
            name = row["name"]
            if name in ("minecraft:add_entity", "minecraft:remove_entities",
                        "minecraft:player_info_update", "minecraft:player_info_remove",
                        "minecraft:set_entity_data", "minecraft:update_attributes"):
                continue
            rel = row["t_ms"] - offset
            # nearest move tick
            best = min(range(len(bm)), key=lambda i: abs(bm[i]["t"] - rel))
            buckets[bm[best]["tick"]].append(row)
        # summarize how many ticks have which packet-type sets
        shapes = Counter()
        for tick, rs in buckets.items():
            shapes[tuple(sorted(r["name"].replace("minecraft:", "") for r in rs))] += 1
        print("A-packet sets per B move tick (top 20):", file=sys.stdout)
        for shape, count in shapes.most_common(20):
            print(f"  {count:4d}  {shape}", file=sys.stdout)
        # ticks with no A packet
        missing = [i for i in range(1, len(bm) + 1) if i not in buckets]
        print(f"B ticks with no A movement packet: {len(missing)}/{len(bm)} "
              f"first={missing[:10]}", file=sys.stdout)


def attribute_order(a_rows):
    for row in a_rows:
        if row.get("dir") == "clientbound" and row.get("name") == "minecraft:update_attributes":
            decoded = decode_packet(row["name"], row["payload"])
            print(f"update_attributes at t={row['t_ms']}: records={decoded['records']}",
                  file=sys.stdout)
            return


def player_info(a_rows, label):
    print(f"-- {label} player_info_update --", file=sys.stdout)
    for row in a_rows:
        if row.get("dir") == "clientbound" and row.get("name") == "minecraft:player_info_update":
            print(f"  t={row['t_ms']:.1f} len={len(row['payload'])} hex={row['payload'].hex()}",
                  file=sys.stdout)


def main():
    for label, pa, pb in [
        ("C1 vanilla", TRACES / "two-c1-A.jsonl", TRACES / "two-c1-B.jsonl"),
        ("C2 vanilla", TRACES / "two-c2-A.jsonl", TRACES / "two-c2-B.jsonl"),
        ("C4 vanilla", TRACES / "two-c4-A.jsonl", TRACES / "two-c4-B.jsonl"),
    ]:
        summarize(label, pa, pb)
    for label, pa in [
        ("C1 vanilla", TRACES / "two-c1-A.jsonl"),
    ]:
        a_rows = rows(pa)
        attribute_order(a_rows)
        player_info(a_rows, label)


if __name__ == "__main__":
    main()