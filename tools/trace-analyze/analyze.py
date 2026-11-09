#!/usr/bin/env python3
"""Aggregate analyzer for the parity JSONL traces (M4.0 round 5).

The trace format is the bot's own JSONL (`t_ms`, `dir`, `state`, `id`,
`name`, `b64`). Every command prints aggregates only: counters and
min/median/max. Raw coordinate or packet lists appear only with `--list`.
Payload bodies are never printed.

    python tools/trace-analyze/analyze.py summary <trace.jsonl> [--list]
    python tools/trace-analyze/analyze.py window  <trace.jsonl> [--list]
    python tools/trace-analyze/analyze.py gap     <trace.jsonl> --at MS
                                                  [--around MS] [--list]
    python tools/trace-analyze/analyze.py outside <trace.jsonl>
                                                  --before SCAN --after SCAN
                                                  [--radius 10] [--list]

`SCAN` is a text file produced by
`python target/parity/chunk_status.py scan <world-dir>`.

Whats each command answers:

* `summary` — what the run was: packet counters by direction/state, span.
* `window`  — the server's loaded-chunk view: centre moves with time, bot
  position and chunk; chunks/forgets between consecutive moves; batch
  sizes and gaps.
* `gap`     — one time window in event order (bot position, centre moves,
  forgets, batches, accepted vs not-yet-acked batches, what the bot sent).
* `outside` — the owner's world criterion: new chunks outside the send
  window (radius `R` around every centre the server set during the run),
  aggregated by `Status` and by Chebyshev distance from the last centre;
  chunks inside the window that changed status/content.
"""
import argparse
import base64
import json
import statistics
import struct
import sys
from collections import Counter, OrderedDict

sys.stdout.reconfigure(encoding='utf-8', errors='replace')

MOVE_POS = 'minecraft:move_player_pos'
MOVE_POS_ROT = 'minecraft:move_player_pos_rot'
TICK_END = 'minecraft:client_tick_end'
BATCH_START = 'minecraft:chunk_batch_start'
BATCH_FINISHED = 'minecraft:chunk_batch_finished'
BATCH_RECEIVED = 'minecraft:chunk_batch_received'
CHUNK = 'minecraft:level_chunk_with_light'
CENTRE = 'minecraft:set_chunk_cache_center'
FORGET = 'minecraft:forget_level_chunk'


def load(path):
    """Rows of one trace, payloads decoded from base64."""
    rows = []
    with open(path, encoding='utf-8', errors='replace') as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            row['payload'] = base64.b64decode(row.get('b64') or '')
            row['t'] = float(row['t_ms'])
            rows.append(row)
    rows.sort(key=lambda r: r['seq'])
    return rows


def varint(buf, pos=0):
    """Signed VarInt (Java's zig-zag-free two's complement)."""
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


def position(row):
    """(x, y, z) of a move packet, or None."""
    body = row['payload']
    if row['name'] in (MOVE_POS, MOVE_POS_ROT) and len(body) >= 24:
        return struct.unpack_from('>ddd', body, 0)
    return None


def chunk_centre(row):
    """(cx, cz) of set_chunk_cache_center, or None."""
    if len(row['payload']) < 2:
        return None
    cx, pos = varint(row['payload'], 0)
    cz, _ = varint(row['payload'], pos)
    return cx, cz


def forget_coords(row):
    """(cx, cz) of forget_level_chunk (two big-endian i32), or None."""
    if len(row['payload']) != 8:
        return None
    return struct.unpack('>ii', row['payload'])


def chunk_of(block):
    return int(block // 16)


def stat_line(label, values, unit=''):
    if not values:
        print(f'{label:<38} n=0')
        return
    print(f'{label:<38} n={len(values):>5}  min {min(values):.1f}{unit}'
          f'  median {statistics.median(values):.1f}{unit}'
          f'  max {max(values):.1f}{unit}')


def batches(rows):
    """Finished chunk batches: (t, chunks, gap from previous finish)."""
    out = []
    count = 0
    for row in rows:
        if row['dir'] != 'clientbound':
            continue
        if row['name'] == BATCH_START:
            count = 0
        elif row['name'] == CHUNK:
            count += 1
        elif row['name'] == BATCH_FINISHED:
            gap = row['t'] - out[-1]['t'] if out else 0.0
            out.append({'t': row['t'], 'chunks': count, 'gap': gap})
            count = 0
    return out


def nearest_position(rows, index):
    """Last bot position at or before rows[index]."""
    for row in reversed(rows[:index + 1]):
        if row['dir'] == 'serverbound':
            pos = position(row)
            if pos is not None:
                return pos
    return None


class Context:
    """Packet-derived landmarks of one trace."""

    def __init__(self, rows):
        self.rows = rows
        self.moves = [(r['t'], position(r)) for r in rows
                      if r['dir'] == 'serverbound' and position(r) is not None]
        self.centres = []
        for index, row in enumerate(rows):
            if row['dir'] == 'clientbound' and row['name'] == CENTRE:
                centre = chunk_centre(row)
                if centre is not None:
                    self.centres.append({
                        't': row['t'],
                        'centre': centre,
                        'pos': nearest_position(rows, index),
                    })
        self.forgets = [(r['t'], forget_coords(r)) for r in rows
                        if r['dir'] == 'clientbound' and r['name'] == FORGET]
        self.batches = batches(rows)
        self.acks = [r['t'] for r in rows
                     if r['dir'] == 'serverbound' and r['name'] == BATCH_RECEIVED]

    def centre_at(self, when):
        """Last centre move at or before `when`."""
        found = None
        for move in self.centres:
            if move['t'] <= when:
                found = move
            else:
                break
        return found

    def position_at(self, when):
        found = None
        for when_move, pos in self.moves:
            if when_move <= when:
                found = pos
            else:
                break
        return found


def cmd_summary(ctx, args):
    rows = ctx.rows
    print(f'trace: {args.trace}')
    print(f'rows: {len(rows)}  span: {rows[-1]["t"] - rows[0]["t"]:.1f} ms'
          f'  first {rows[0]["t"]:.1f}  last {rows[-1]["t"]:.1f}')
    by_state = Counter((r['dir'], r['state']) for r in rows)
    for (direction, state), count in sorted(by_state.items()):
        print(f'  {direction:<13} {state:<14} {count}')
    print('packets:')
    by_name = Counter(r['name'] or f'id {r["id"]}' for r in rows)
    for name, count in by_name.most_common():
        print(f'  {name:<44} {count}')
    print(f'moves: {len(ctx.moves)}  last position: '
          f'{ctx.moves[-1][1] if ctx.moves else None}')
    chunks = [r for r in rows if r['name'] == CHUNK]
    print(f'chunks: {len(chunks)}  chunk bytes: {sum(len(r["payload"]) for r in chunks)}')
    print(f'centres: {len(ctx.centres)}  forgets: {len(ctx.forgets)}'
          f'  batches: {len(ctx.batches)}')
    if args.list:
        for when, pos in ctx.moves:
            print(f'  move t={when:.1f} pos={pos}')
        for move in ctx.centres:
            print(f'  centre t={move["t"]:.1f} {move["centre"]} pos={move["pos"]}')
        for when, coords in ctx.forgets:
            print(f'  forget t={when:.1f} {coords}')


def cmd_window(ctx, args):
    print(f'trace: {args.trace}')
    print('centre moves (time, centre, bot position, chunks and forgets until '
          'the next move):')
    for index, move in enumerate(ctx.centres):
        end = (ctx.centres[index + 1]['t'] if index + 1 < len(ctx.centres)
               else float('inf'))
        got = [b for b in ctx.batches if move['t'] <= b['t'] < end]
        chunks = sum(b['chunks'] for b in got)
        forgot = sum(1 for when, _ in ctx.forgets if move['t'] <= when < end)
        pos = move['pos']
        chunk = None if pos is None else (chunk_of(pos[0]), chunk_of(pos[2]))
        shown = 'none' if pos is None else f'({pos[0]:.1f}, {pos[2]:.1f})'
        print(f'  t={move["t"]:>8.1f}  centre={move["centre"]}  pos={shown}'
              f'  chunk={chunk}  batches={len(got)}  chunks={chunks}'
              f'  forgets={forgot}')
    if ctx.centres:
        xs = [c for _, c in ctx.forgets if c]
        if xs:
            print(f'forget bbox: x {min(v[0] for v in xs)}..{max(v[0] for v in xs)}'
                  f'  z {min(v[1] for v in xs)}..{max(v[1] for v in xs)}')
    stat_line('batch size, chunks', [b['chunks'] for b in ctx.batches])
    stat_line('batch gap, ms', [b['gap'] for b in ctx.batches[1:]])
    gaps = [(b['t'], b['gap']) for b in ctx.batches[1:] if b['gap'] >= 500]
    print(f'gaps >= 500 ms: {len(gaps)}')
    for when, gap in sorted(gaps, key=lambda item: -item[1])[:5]:
        pos = ctx.position_at(when)
        centre = ctx.centre_at(when)
        print(f'  gap {gap:>7.0f} ms at t={when:.1f}  pos={pos}  '
              f'centre={centre["centre"] if centre else None}')
    if args.list:
        for batch in ctx.batches:
            print(f'  batch t={batch["t"]:.1f} chunks={batch["chunks"]} gap={batch["gap"]:.1f}')


def cmd_gap(ctx, args):
    low, high = args.at - args.around, args.at + args.around
    rows = [r for r in ctx.rows if low <= r['t'] <= high]
    print(f'trace: {args.trace}  window: {low:.0f}..{high:.0f} ms ({len(rows)} packets)')
    print('events:')
    for row in rows:
        detail = ''
        if row['name'] == CENTRE:
            detail = f' centre={chunk_centre(row)}'
        elif row['name'] == FORGET:
            detail = f' chunk={forget_coords(row)}'
        elif row['name'] in (MOVE_POS, MOVE_POS_ROT):
            pos = position(row)
            detail = f' pos=({pos[0]:.1f}, {pos[2]:.1f})'
        elif row['name'] == BATCH_RECEIVED:
            detail = f' pace={struct.unpack(">f", row["payload"])[0]:.4f}'
        print(f'  t={row["t"]:>9.1f} {row["dir"]:<12} {row["name"]:<40}{detail}')
    print('aggregates in the window:')
    finished = [r for r in rows if r['dir'] == 'clientbound' and r['name'] == BATCH_FINISHED]
    acks = [r for r in rows if r['dir'] == 'serverbound' and r['name'] == BATCH_RECEIVED]
    chunks = [r for r in rows if r['name'] == CHUNK]
    print(f'  batches finished: {len(finished)}  acks sent: {len(acks)}  '
          f'chunks: {len(chunks)}  chunk bytes: {sum(len(r["payload"]) for r in chunks)}')
    print(f'  moves sent: {sum(1 for r in rows if r["name"] in (MOVE_POS, MOVE_POS_ROT))}'
          f'  tick ends: {sum(1 for r in rows if r["name"] == TICK_END)}')
    # Batches finished before the window that still had no ack when it began.
    before = [r['t'] for r in ctx.batches if r['t'] < low]
    acked_before = sum(1 for t in ctx.acks if t < low)
    print(f'  batches finished before window: {len(before)}  acked before: {acked_before}'
          f'  awaiting ack at window start: {len(before) - acked_before}')
    if args.list:
        for row in rows:
            print(f'  {row["seq"]} {row["t"]:.1f} {row["dir"]} {row["name"]}')


def read_scan(path):
    """(x, z) -> (status, sections, block_light, sky_light, heightmap)."""
    out = {}
    with open(path, encoding='utf-8', errors='replace') as handle:
        for line in handle:
            line = line.strip()
            if not line.startswith('('):
                continue
            head, *fields = line.split(') status=')
            coords = head.lstrip('(').split(',')
            x, z = int(coords[0]), int(coords[1])
            parts = dict(item.split('=') for item in fields[0].split())
            out[(x, z)] = (
                parts['status'],
                int(parts['secs']),
                int(parts['block']),
                int(parts['sky']),
                parts['hm'],
            )
    return out


def window_cells(centres, radius):
    cells = set()
    for cx, cz in centres:
        for x in range(cx - radius, cx + radius + 1):
            for z in range(cz - radius, cz + radius + 1):
                cells.add((x, z))
    return cells


def chebyshev(a, b):
    return max(abs(a[0] - b[0]), abs(a[1] - b[1]))


def cmd_outside(ctx, args):
    before = read_scan(args.before)
    after = read_scan(args.after)
    centres = [move['centre'] for move in ctx.centres]
    cells = window_cells(centres, args.radius)
    last = centres[-1] if centres else (0, 0)
    new = sorted(set(after) - set(before))
    inside_new = [key for key in new if key in cells]
    outside = [key for key in new if key not in cells]
    print(f'trace: {args.trace}  radius: {args.radius}  centres: {len(centres)}'
          f'  last centre: {last}')
    print(f'before: {len(before)} chunks  after: {len(after)} chunks'
          f'  new: {len(new)}  window cells: {len(cells)}')
    print(f'new inside window: {len(inside_new)}  new outside window: {len(outside)}')
    if outside:
        counts = Counter(after[key][0] for key in outside)
        print('outside chunks by Status:')
        for status, count in counts.most_common():
            print(f'  {status:<32} {count}')
        dist = [chebyshev(key, last) for key in outside]
        print(f'outside distance from last centre (chunks):'
              f' min {min(dist)}  median {statistics.median(dist)}  max {max(dist)}')
        buckets = Counter(d // 2 * 2 for d in dist)
        print('outside distance histogram (2-chunk buckets):')
        for bucket in sorted(buckets):
            print(f'  {bucket:>3}..{bucket + 1:<3} {buckets[bucket]}')
    changed = []
    gone = []
    for key in sorted(set(before) & set(after)):
        if key not in cells:
            continue
        if before[key][0] != after[key][0]:
            changed.append((key, before[key][0], after[key][0]))
        elif before[key][4] != after[key][4] or before[key][2:4] != after[key][2:4]:
            changed.append((key, 'content', 'content'))
    for key in sorted(set(before) - set(after)):
        if key in cells:
            gone.append(key)
    print(f'inside window: changed status/content {len(changed)}  gone {len(gone)}')
    if args.list:
        for key, old, new_status in changed:
            print(f'  changed {key} {old} -> {new_status}')
        for key in gone:
            print(f'  gone {key}')
        for key in outside:
            print(f'  outside {key} status={after[key][0]} dist={chebyshev(key, last)}')
        for key in inside_new:
            print(f'  inside-new {key} status={after[key][0]}')


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--list', action='store_true',
                        help='print raw lists (default: aggregates only)')
    sub = parser.add_subparsers(dest='command', required=True)

    summary = sub.add_parser('summary')
    summary.add_argument('trace')
    summary.set_defaults(func=cmd_summary)

    window = sub.add_parser('window')
    window.add_argument('trace')
    window.set_defaults(func=cmd_window)

    gap = sub.add_parser('gap')
    gap.add_argument('trace')
    gap.add_argument('--at', type=float, required=True)
    gap.add_argument('--around', type=float, default=500.0)
    gap.set_defaults(func=cmd_gap)

    outside = sub.add_parser('outside')
    outside.add_argument('trace')
    outside.add_argument('--before', required=True)
    outside.add_argument('--after', required=True)
    outside.add_argument('--radius', type=int, default=10)
    outside.set_defaults(func=cmd_outside)

    args = parser.parse_args(argv)
    rows = load(args.trace)
    args.func(Context(rows), args)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))