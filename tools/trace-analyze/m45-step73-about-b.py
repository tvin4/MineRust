"""M4.5 step 7.3: dump the packets about bot B in the C5/C6 traces.

Reads bot A's JSONL trace, finds B's entity id from the `add_entity` whose
UUID is B's offline UUID, and prints every later clientbound packet that
carries that entity id, with hex bytes. Used to see the exact packets behind
the step 7.3 parity diffs (no conclusion of its own).

Usage:
    python tools/trace-analyze/m45-step73-about-b.py C:\\MineRust-data\\traces\\two-c5-A.jsonl
"""

from __future__ import annotations

import base64
import json
import struct
import sys
import uuid

NAMESPACE = uuid.UUID("6ba7b810-9dad-11d1-80b4-00c04fd430c8")


def offline_uuid(name: str) -> bytes:
    """Java `UUID.nameUUIDFromBytes` of `OfflinePlayer:<name>`."""
    digest = uuid.uuid3  # not the Java algorithm; computed below
    del digest
    import hashlib

    data = hashlib.md5(("OfflinePlayer:" + name).encode("utf-8")).digest()
    raw = bytearray(data)
    raw[6] = (raw[6] & 0x0F) | 0x30
    raw[8] = (raw[8] & 0x3F) | 0x80
    return bytes(raw)


def read_varint(buf: bytes, pos: int) -> tuple[int, int]:
    value = 0
    shift = 0
    while True:
        byte = buf[pos]
        pos += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return value, pos
        shift += 7


def packets(path: str):
    with open(path, "r", encoding="utf-8") as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            if row.get("dir") != "clientbound":
                continue
            yield row, base64.b64decode(row["b64"])


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    path = sys.argv[1]
    target = offline_uuid("TraceBotB")
    b_id = None
    for row, payload in packets(path):
        name = row.get("name") or ""
        if name == "minecraft:add_entity" and b_id is None:
            _, pos = read_varint(payload, 0)
            if payload[pos : pos + 16] == target:
                b_id, _ = read_varint(payload, 0)
                print(f"B entity id = {b_id} (add_entity at t={row['t_ms']} ms)")
    if b_id is None:
        print("B never spawned")
        return 1
    for row, payload in packets(path):
        try:
            entity_id, _ = read_varint(payload, 0)
        except IndexError:
            continue
        if entity_id != b_id:
            continue
        name = row.get("name") or "?"
        hexed = " ".join(f"{byte:02x}" for byte in payload)
        print(f"{row['seq']:>5} {row['t_ms']:>9.3f} {name:<32} {hexed}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
