"""M4.5 step 8: parse the observer's trace (client B) and print, per scenario
(S1..S4), every clientbound packet that speaks about player A with time and
field-level detail: add_entity (angle bytes + head yaw), rotate_head,
set_entity_data (index, type, value), animate, entity_event, update_attributes.

Usage:
    python m45-step80-scenarios.py --dir C:\\MineRust-data\\traces --scenario s1
    python m45-step80-scenarios.py --dir C:\\MineRust-data\\traces --all

Expects <dir>/<scenario>-A.jsonl and <dir>/<scenario>-B.jsonl (files written by
mcproxy, one JSON object per line: t_ms, seq, dir, id, name, state, b64).
The A trace is used only to learn player A's entity id and UUID; all printed
events come from the B trace (the observer's clientbound stream).

Field names for the decoded layouts come from crates/protocol/src/packets/*
(the project's codecs) and from C:\\MineRust-data\\reports\\packets.json for
packet ids. packets.json carries only protocol_id values - it has no field
names - so anything packets.json cannot confirm is printed as raw bytes and
marked "не установлено". Unknown metadata types abort the value and print the
raw remainder; nothing is guessed.
"""
import argparse
import base64
import json
import struct
import sys
from pathlib import Path

PACKETS_JSON = Path(r"C:\MineRust-data\reports\packets.json")

SCENARIOS = {
    "s1": "A stands with a non-zero yaw/pitch, then B joins",
    "s2": "A presses Shift (sneak)",
    "s3": "A swings three times in the air, then breaks a block",
    "s4": "A sprints on land, /tp into water, swims while sprinting",
}


# ---------------------------------------------------------------------------
# varint / wire helpers
# ---------------------------------------------------------------------------

def varint(b, p=0):
    v = s = 0
    while True:
        if p >= len(b):
            raise EOFError("varint past end")
        x = b[p]
        p += 1
        v |= (x & 0x7F) << s
        if not x & 0x80:
            return v, p
        s += 7


def f64(b, p):
    return struct.unpack_from(">d", b, p)[0], p + 8


def f32(b, p):
    return struct.unpack_from(">f", b, p)[0], p + 4


def uuid_hex(b, p):
    return b[p:p + 16].hex(), p + 16


def ang(byte):
    """Angle byte to degrees is not confirmed; show both raw and degrees."""
    return "%02x (%.1f deg, mapping unverified)" % (byte, byte * 360.0 / 256.0)


# ---------------------------------------------------------------------------
# metadata value sizes (from trace observations only - see module docstring)
# ---------------------------------------------------------------------------
# type id -> ("label", byte size or "varint" / "none")
# Semantics are NOT established by packets.json (no field names there); the
# labels below are hints and every value is printed as raw hex as well.
# Sizes come only from trace observations (docs/PROGRESS.md step 8 / m45 notes):
#   body 09 03 41 a0 00 00 10 00 7f ff -> (9, type 3) = 4 bytes 41 a0 00 00
#   = f32 20.0 (health, the standing-player value), then (16, type 0) byte 0x7f,
#   terminator ff. (0,0) flags, (8,7) itemstack, (15,0), (16,0)/(16,8), (18,1),
#   and goat pose (6,20) values 6/0 are the other observed pairs.
META_TYPES = {
    0: ("byte", 1),
    1: ("varint", "varint"),
    3: ("float f32 (observed 20.0=health)", 4),
    7: ("itemstack (observed, size unknown)", "none"),
    8: ("type 8 (observed at idx 16, size unknown)", "none"),
    16: ("type 16 (observed at idx 16, size unknown)", "none"),
    18: ("type 18 (observed at idx 18, size unknown)", "none"),
    20: ("pose enum (observed at idx 6)", "varint"),
}


def parse_metadata(body):
    """Yield (index, type_id, label, value_repr, raw_hex) until end of body.

    Body is the set_entity_data payload after entity id and without the 0xFF
    terminator. A record is: u8 index, VarInt type, then the type's value.
    Types whose size is unknown ("none") stop the parse and the remainder is
    printed raw - no guessing.
    """
    out = []
    p = 0
    while p < len(body):
        idx = body[p]
        p += 1
        try:
            t, p = varint(body, p)
        except EOFError:
            out.append((idx, None, "truncated", "", body[p - 1:].hex(" ")))
            break
        label, size = META_TYPES.get(t, ("unknown", "none"))
        if size == "none":
            out.append((idx, t, label, "<raw remainder>", body[p:].hex(" ")))
            break
        start = p
        if size == "varint":
            try:
                v, p = varint(body, p)
            except EOFError:
                out.append((idx, t, label, "truncated", body[start:].hex(" ")))
                break
            repr_v = str(v)
        else:
            if p + size > len(body):
                out.append((idx, t, label, "truncated", body[start:].hex(" ")))
                break
            raw = body[p:p + size]
            if size == 1:
                repr_v = "%d / 0x%02x" % (raw[0], raw[0])
            elif size == 4:
                repr_v = "%g" % struct.unpack(">f", raw)[0]
            else:
                repr_v = raw.hex(" ")
            p += size
        out.append((idx, t, label, repr_v, body[start:p].hex(" ")))
    return out


# ---------------------------------------------------------------------------
# packet decoders (layouts from crates/protocol/src/packets, ids packets.json)
# ---------------------------------------------------------------------------

def dec_add_entity(b):
    p = 0
    eid, p = varint(b, p)
    u, p = uuid_hex(b, p)
    etype, p = varint(b, p)
    x, p = f64(b, p)
    y, p = f64(b, p)
    z, p = f64(b, p)
    a0, a1, a2 = b[p], b[p + 1], b[p + 2]
    p += 3
    data, p = varint(b, p)
    tail = b[p] if p < len(b) else None
    return [
        ("entity_id", str(eid)),
        ("uuid", u),
        ("entity_type", str(etype)),
        ("x/y/z", "%.3f %.3f %.3f" % (x, y, z)),
        ("angle_bytes[0..2] (order не установлено)", "%s %s %s" % (ang(a0), ang(a1), ang(a2))),
        ("data (VarInt, semantics не установлено)", str(data)),
        ("tail (semantics не установлено)", str(tail)),
    ]


def dec_rotate_head(b):
    eid, p = varint(b, 0)
    return [
        ("entity_id", str(eid)),
        ("head_yaw byte", ang(b[p]) if p < len(b) else "truncated"),
    ]


def dec_set_entity_data(b):
    eid, p = varint(b, 0)
    body = b[p:]
    term = ""
    if body.endswith(b"\xff"):
        body = body[:-1]
        term = " (0xFF terminator present)"
    rows = [("entity_id", str(eid)), ("terminator", "0xFF" + term)]
    for idx, t, label, repr_v, raw in parse_metadata(body):
        rows.append(("meta[%d] type=%s (%s)" % (idx, t, label),
                     "%s   raw: %s" % (repr_v, raw)))
    return rows


def dec_update_attributes(b):
    eid, p = varint(b, 0)
    count, p = varint(b, p)
    rows = [("entity_id", str(eid)), ("count", str(count))]
    for i in range(count):
        key, p = varint(b, p)
        base, p = f64(b, p)
        mods, p = varint(b, p)
        row = "key=%d base=%g modifiers=%d" % (key, base, mods)
        for _ in range(mods):
            mu, p = uuid_hex(b, p)
            amount, p = f64(b, p)
            op = b[p]
            p += 1
            row += " | mod uuid=%s amount=%g op=%d" % (mu, amount, op)
        rows.append(("attr[%d]" % i, row))
    return rows


def dec_varint2(b):
    """animate / entity_event have no layout in crates/protocol: raw + two
    leading VarInts shown as hints, marked unverified."""
    rows = [("raw", b.hex(" "))]
    try:
        v0, p = varint(b, 0)
        rows.append(("first VarInt (id? - не установлено)", str(v0)))
        v1, p2 = varint(b, p)
        rows.append(("second VarInt (kind? - не установлено)", str(v1)))
    except EOFError:
        rows.append(("varint", "truncated"))
    return rows


DECODERS = {
    "minecraft:add_entity": dec_add_entity,
    "minecraft:rotate_head": dec_rotate_head,
    "minecraft:set_entity_data": dec_set_entity_data,
    "minecraft:update_attributes": dec_update_attributes,
    "minecraft:animate": dec_varint2,
    "minecraft:entity_event": dec_varint2,
}

WATCH = set(DECODERS)


# ---------------------------------------------------------------------------
# trace loading
# ---------------------------------------------------------------------------

def load_rows(path):
    rows = []
    with open(path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                r = json.loads(line)
            except json.JSONDecodeError:
                continue
            r["payload"] = base64.b64decode(r.get("b64") or "")
            rows.append(r)
    rows.sort(key=lambda r: r.get("t_ms", 0))
    return rows


def a_identities(a_rows, b_rows):
    """Player A's identities: list of (entity_id, uuid_hex).

    The observer's trace (B) is authoritative: player A is the only entity the
    server spawns there with entity_type 159 (the player type, registries.json).
    There can be more than one (a respawn re-spawns A with a new entity id).
    Player A's own trace (A) has no add_entity for A (a client never sees
    itself), so it is only used as a fallback.
    """
    out = []
    for rows in (b_rows, a_rows):
        for r in rows:
            if r.get("name") != "minecraft:add_entity" or r.get("dir") != "clientbound":
                continue
            b = r["payload"]
            try:
                eid, p = varint(b, 0)
                u, p = uuid_hex(b, p)
                etype, _ = varint(b, p)
            except EOFError:
                continue
            if etype == 159:
                out.append((eid, u))
        if out:
            break
    return out


def print_rows(t_ms, name, rows):
    print("  [%9.3f ms] %s" % (t_ms, name))
    for k, v in rows:
        print("      %-42s %s" % (k, v))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", default=r"C:\MineRust-data\traces")
    ap.add_argument("--scenario", choices=sorted(SCENARIOS))
    ap.add_argument("--all", action="store_true")
    args = ap.parse_args()
    wanted = sorted(SCENARIOS) if args.all else [args.scenario]
    if not wanted:
        ap.error("give --scenario or --all")
    tdir = Path(args.dir)
    ids = load_packet_ids()
    for sc in wanted:
        pa = tdir / ("%s-A.jsonl" % sc)
        pb = tdir / ("%s-B.jsonl" % sc)
        if not pb.exists():
            print("== %s: MISSING %s (skip)" % (sc, pb))
            continue
        a_rows = load_rows(pa) if pa.exists() else []
        b_rows = load_rows(pb)
        a_ids = a_identities(a_rows, b_rows)
        print()
        print("== scenario %s == %s" % (sc, SCENARIOS[sc]))
        print("   observer trace: %s (%d rows)" % (pb, len(b_rows)))
        if a_ids:
            for eid, u in a_ids:
                print("   player A: entity_id=%d uuid=%s" % (eid, u))
            aeids = {eid for eid, _ in a_ids}
        else:
            aeids = None
            print("   player A: entity id NOT established from %s - "
                  "printing all entities" % pb.name)
        shown = 0
        for r in b_rows:
            if r.get("dir") != "clientbound":
                continue
            name = r.get("name")
            if name not in WATCH:
                continue
            b = r["payload"]
            # filter to player A when known (events carry entity id first)
            if aeids is not None and name != "minecraft:add_entity":
                try:
                    first, _ = varint(b, 0)
                except EOFError:
                    continue
                if first not in aeids:
                    continue
            if aeids is not None and name == "minecraft:add_entity":
                try:
                    first, _ = varint(b, 0)
                except EOFError:
                    continue
                if first not in aeids:
                    continue
            dec = DECODERS[name]
            try:
                rows = dec(b)
            except Exception as exc:  # never guess past a malformed payload
                rows = [("decode error", repr(exc)), ("raw", b.hex(" "))]
            pid = ids.get(name)
            print_rows(r.get("t_ms", 0.0),
                       "%s (id=%s)" % (name, pid if pid is not None else "?"),
                       rows)
            shown += 1
        if shown == 0:
            print("   (no watched clientbound packets for player A)")


def load_packet_ids():
    try:
        data = json.loads(PACKETS_JSON.read_text(encoding="utf-8"))
    except OSError:
        return {}
    out = {}
    play = data.get("play", {})
    for side in ("clientbound", "serverbound"):
        for name, rec in play.get(side, {}).items():
            out[name] = rec.get("protocol_id")
    return out


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    main()
