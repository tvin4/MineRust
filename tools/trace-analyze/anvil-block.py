#!/usr/bin/env python3
"""Read block states out of a vanilla Anvil region file (read-only check).

Used to settle a black-box question from the S10 compare: does the vanilla
world really contain water where our in-memory store has air? The chunk data
on disk is the vanilla server's own output, so this is evidence, not a guess.

Usage:
  python tools/trace-analyze/anvil-block.py <region.mca> '[[x,y,z], ...]'

The region file is the vanilla run's saved world, e.g.
E:/MineRust-data/vanilla/world/dimensions/minecraft/overworld/region/r.0.-1.mca.
Only the 1.18+ ``sections[].block_states`` palette form is decoded.
"""
import gzip
import json
import struct
import sys
import zlib


def read_value(data, pos, tag):
    """Reads one NBT payload of the given tag; returns (value, new_pos)."""
    if tag == 0:
        return None, pos
    if tag == 1:
        return struct.unpack_from('>b', data, pos)[0], pos + 1
    if tag == 2:
        return struct.unpack_from('>h', data, pos)[0], pos + 2
    if tag == 3:
        return struct.unpack_from('>i', data, pos)[0], pos + 4
    if tag == 4:
        return struct.unpack_from('>q', data, pos)[0], pos + 8
    if tag == 5:
        return struct.unpack_from('>f', data, pos)[0], pos + 4
    if tag == 6:
        return struct.unpack_from('>d', data, pos)[0], pos + 8
    if tag == 7:
        n = struct.unpack_from('>i', data, pos)[0]
        return data[pos + 4:pos + 4 + n], pos + 4 + n
    if tag == 8:
        n = struct.unpack_from('>H', data, pos)[0]
        pos += 2
        return data[pos:pos + n].decode('utf-8'), pos + n
    if tag == 9:
        item = data[pos]
        n = struct.unpack_from('>i', data, pos + 1)[0]
        pos += 5
        out = []
        for _ in range(n):
            value, pos = read_value(data, pos, item)
            out.append(value)
        return out, pos
    if tag == 10:
        out = {}
        while data[pos] != 0:
            name, value, pos = read_named(data, pos)
            out[name] = value
        return out, pos + 1
    if tag == 11:
        n = struct.unpack_from('>i', data, pos)[0]
        return list(struct.unpack_from('>%di' % n, data, pos + 4)), pos + 4 + 4 * n
    if tag == 12:
        n = struct.unpack_from('>i', data, pos)[0]
        return list(struct.unpack_from('>%dq' % n, data, pos + 4)), pos + 4 + 8 * n
    raise ValueError('unknown tag %d' % tag)


def read_named(data, pos):
    """Reads tag byte + name + payload; returns (name, value, new_pos)."""
    tag = data[pos]
    pos += 1
    n = struct.unpack_from('>H', data, pos)[0]
    pos += 2
    name = data[pos:pos + n].decode('utf-8')
    pos += n
    value, pos = read_value(data, pos, tag)
    return name, value, pos


def load_chunk(region_path, chunk_x, chunk_z):
    with open(region_path, 'rb') as handle:
        blob = handle.read()
    index = ((chunk_x & 31) + (chunk_z & 31) * 32) * 4
    offset = (struct.unpack_from('>I', blob, index)[0] >> 8) * 4096
    length = struct.unpack_from('>I', blob, offset)[0]
    compression = blob[offset + 4]
    payload = blob[offset + 5:offset + 4 + length]
    raw = gzip.decompress(payload) if compression == 1 else zlib.decompress(payload)
    _, value, _ = read_named(raw, 0)
    return value


def block_at(chunk, x, y, z):
    for section in chunk.get('sections', []):
        if section['Y'] != (y >> 4) or 'block_states' not in section:
            continue
        states = section['block_states']
        palette = states['palette']
        data = states.get('data')
        index = (x & 15) + (z & 15) * 16 + (y & 15) * 256
        if data is None:
            return palette[0]
        bits = max(4, (len(palette) - 1).bit_length())
        per_long = 64 // bits
        value = (data[index // per_long] >> ((index % per_long) * bits)) & ((1 << bits) - 1)
        return palette[value]
    return None


def main():
    region = sys.argv[1]
    for x, y, z in json.loads(sys.argv[2]):
        chunk = load_chunk(region, x >> 4, z >> 4)
        print('(%d,%d,%d) -> %s' % (x, y, z, json.dumps(block_at(chunk, x, y, z))))


if __name__ == '__main__':
    main()
