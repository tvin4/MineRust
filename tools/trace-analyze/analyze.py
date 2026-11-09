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
    python tools/trace-analyze/analyze.py shape   <trace.jsonl>
                                                  [--square 10] [--list]

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
* `shape`   — the form of the send window per centre shift: the loaded set
  (delivered minus forgotten), the full set's per-row widths, and per shift
  the arriving/forgotten set relative to the new/old centre, next to the
  `2*square+1` edge a `±square` window would predict. Also counts the
  `player_position` packets (a rollback detector).
"""
import argparse
import base64
import json
import re
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
POSITION = 'minecraft:player_position'


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
    """(cx, cz) of forget_level_chunk, or None.

    The payload is the packed `ChunkPos` long (8 B). Read as two big-endian
    i32 it is `(z, x)`: the high word is z, the low word is x. Round 5
    documented "two big-endian i32" but not the word order; the traces show
    the first word is z (the trailing edge would otherwise fall outside the
    loaded set).
    """
    if len(row['payload']) != 8:
        return None
    z, x = struct.unpack('>ii', row['payload'])
    return x, z


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


def chunk_coords(row):
    """(cx, cz) of level_chunk_with_light (two big-endian i32), or None."""
    if row['name'] != CHUNK or len(row['payload']) < 8:
        return None
    return struct.unpack('>ii', row['payload'][:8])


def row_widths(cells, centre, axis=0):
    """axis 0: dx -> sorted dz; axis 1: dz -> sorted dx, relative to centre."""
    out = {}
    for (x, z) in cells:
        key = x - centre[0] if axis == 0 else z - centre[1]
        value = z - centre[1] if axis == 0 else x - centre[0]
        out.setdefault(key, []).append(value)
    return {key: sorted(values) for key, values in out.items()}


def compact_counts(rows):
    """`dx:n dx:n ...` in dx order."""
    return ' '.join(f'{dx}:{len(dzs)}' for dx, dzs in sorted(rows.items()))


def compact_spans(rows):
    """`dx:dz0..dz1 dx:...` in dx order."""
    return ' '.join(f'{dx}:{dzs[0]}..{dzs[-1]}' for dx, dzs in sorted(rows.items()))


def square_edge(radius, dx, dz):
    """New cells of a `±radius` square moved by (dx, dz)."""
    span = abs(dx) + abs(dz)
    return (2 * radius + 1) * span - abs(dx) * abs(dz)


def loaded_sets(ctx):
    """Per centre interval: delivered/forgotten sets and the running loaded set."""
    out = []
    loaded = set()
    centres = ctx.centres
    for index, move in enumerate(centres):
        end = centres[index + 1]['t'] if index + 1 < len(centres) else float('inf')
        delivered_rows = [r for r in ctx.rows
                          if r['dir'] == 'clientbound' and r['name'] == CHUNK
                          and move['t'] <= r['t'] < end]
        forgotten_rows = [r for r in ctx.rows
                          if r['dir'] == 'clientbound' and r['name'] == FORGET
                          and move['t'] <= r['t'] < end]
        delivered = {chunk_coords(r) for r in delivered_rows}
        forgotten = {forget_coords(r) for r in forgotten_rows}
        before = len(loaded)
        loaded |= delivered
        loaded -= forgotten
        delivered_t = [r['t'] for r in delivered_rows]
        out.append({
            'index': index,
            'move': move,
            'delivered': delivered,
            'forgotten': forgotten,
            'before': before,
            'after': len(loaded),
            'loaded': set(loaded),
            'span_t': (min(delivered_t), max(delivered_t)) if delivered_t else None,
        })
    return out


def cmd_shape(ctx, args):
    """Window form per centre shift (see module docstring)."""
    print(f'trace: {args.trace}')
    positions = [(r['seq'], r['t'], r['payload']) for r in ctx.rows
                 if r['dir'] == 'clientbound' and r['name'] == POSITION]
    print(f'player_position packets: {len(positions)}', end='')
    if positions:
        seq, when, payload = positions[0]
        _, at = varint(payload, 0) if payload else (0, 0)
        coords = (struct.unpack_from('>ddd', payload, at)
                  if len(payload) >= at + 24 else None)
        print(f'  first seq={seq} t={when:.1f} pos={coords}')
    else:
        print()
    shifts = loaded_sets(ctx)
    print(f'centre moves: {len(ctx.centres)}')
    if not shifts:
        return
    final = shifts[-1]
    centre = final['move']['centre']
    rows = row_widths(final['loaded'], centre)
    widths = sorted({len(dzs) for dzs in rows.values()})
    print(f'loaded window at last centre {centre}: {final["after"]} chunks'
          f'  row widths seen: {widths}')
    print(f'  per-dx width: {compact_counts(rows)}')
    print(f'  per-dz width: {compact_counts(row_widths(final["loaded"], centre, axis=1))}')
    full = (2 * max(abs(dx) for dx in rows) + 1) ** 2
    print(f'  a full square of that half-width would be {full};'
          f' missing {full - final["after"]} (corners)')
    for shift in shifts:
        index = shift['index']
        move = shift['move']
        prev = ctx.centres[index - 1]['centre'] if index > 0 else None
        delivered = shift['delivered']
        forgotten = shift['forgotten']
        span = f' deliveries t={shift["span_t"][0]:.0f}..{shift["span_t"][1]:.0f}' if shift['span_t'] else ''
        print(f'-- shift c{index} {prev}->{move["centre"]} t={move["t"]:.1f}'
              f'  arrived {len(delivered)} forgotten {len(forgotten)}'
              f'  loaded {shift["before"]}->{shift["after"]}{span}')
        if prev is None:
            continue
        dx = move['centre'][0] - prev[0]
        dz = move['centre'][1] - prev[1]
        arrived = row_widths(delivered, move['centre'])
        left = row_widths(forgotten, prev)
        expect = square_edge(args.square, dx, dz)
        print(f'   shift=({dx},{dz})  arriving rel new: {compact_counts(arrived)}')
        print(f'                          spans: {compact_spans(arrived)}')
        print(f'   forgotten rel old: {compact_counts(left)}  spans: {compact_spans(left)}')
        print(f'   a ±{args.square} square edge would be {expect}; observed {len(delivered)}')
        if args.list:
            print(f'   arrived: {sorted(delivered)}')
            print(f'   forgotten: {sorted(forgotten)}')


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
    pattern = re.compile(
        r'\(\s*(-?\d+)\s*,\s*(-?\d+)\)\s+status=(\S+)\s+secs=(\d+)'
        r'\s+block=\s*(\d+)\s+sky=\s*(\d+)\s+hm=(\S+)'
    )
    out = {}
    with open(path, encoding='utf-8', errors='replace') as handle:
        for line in handle:
            match = pattern.search(line)
            if not match:
                continue
            x, z, status, secs, block, sky, hm = match.groups()
            out[(int(x), int(z))] = (
                status,
                int(secs),
                int(block),
                int(sky),
                hm,
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
    for key in sorted(set(before) & set(after)):
        if key not in cells:
            continue
        old, new = before[key], after[key]
        if old[0] != new[0]:
            changed.append((key, old[0], new[0], 'status'))
        elif old[4] != new[4] or old[2:4] != new[2:4]:
            changed.append((key, old[0], new[0], 'content'))
    gone = [key for key in sorted(set(before) - set(after)) if key in cells]
    inside_before = [key for key in before if key in cells]
    ready_before = [key for key in inside_before if before[key][0] == 'minecraft:full']
    status_changed = [item for item in changed if item[3] == 'status']
    content_changed = [item for item in changed if item[3] == 'content']
    ready_changed = [item for item in changed if item[1] == 'minecraft:full']
    print(f'inside window: before {len(inside_before)} chunks '
          f'(full before {len(ready_before)}); status changed {len(status_changed)}; '
          f'content-only changed {len(content_changed)}; gone {len(gone)}')
    if status_changed:
        transitions = Counter((old, new) for _, old, new, _ in status_changed)
        print('  status transitions inside window:')
        for (old, new), count in transitions.most_common():
            print(f'    {old:<32} -> {new:<20} {count}')
    print(f'  chunks that were full before and changed: {len(ready_changed)}')
    if content_changed:
        hm_only = light_only = both = secs = 0
        for key in [item[0] for item in content_changed]:
            old, new = before[key], after[key]
            hm = old[4] != new[4]
            light = old[2:4] != new[2:4]
            if old[1] != new[1]:
                secs += 1
            if hm and light:
                both += 1
            elif hm:
                hm_only += 1
            elif light:
                light_only += 1
        print(f'  content changes: heightmap only {hm_only}, light only '
              f'{light_only}, both {both}, section count changed {secs}')
    if args.list:
        for key, old, new, kind in changed:
            print(f'  changed {key} {kind}: {before[key]} -> {after[key]}')
        for key in gone:
            print(f'  gone {key}')
        for key in outside:
            print(f'  outside {key} status={after[key][0]} dist={chebyshev(key, last)}')
        for key in inside_new:
            print(f'  inside-new {key} status={after[key][0]}')


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = parser.add_subparsers(dest='command', required=True)

    summary = sub.add_parser('summary')
    summary.add_argument('trace')
    summary.add_argument('--list', action='store_true',
                         help='print raw lists (default: aggregates only)')
    summary.set_defaults(func=cmd_summary)

    window = sub.add_parser('window')
    window.add_argument('trace')
    window.add_argument('--list', action='store_true',
                        help='print raw lists (default: aggregates only)')
    window.set_defaults(func=cmd_window)

    gap = sub.add_parser('gap')
    gap.add_argument('trace')
    gap.add_argument('--at', type=float, required=True)
    gap.add_argument('--around', type=float, default=500.0)
    gap.add_argument('--list', action='store_true',
                     help='print raw lists (default: aggregates only)')
    gap.set_defaults(func=cmd_gap)

    outside = sub.add_parser('outside')
    outside.add_argument('trace')
    outside.add_argument('--before', required=True)
    outside.add_argument('--after', required=True)
    outside.add_argument('--radius', type=int, default=10)
    outside.add_argument('--list', action='store_true',
                         help='print raw lists (default: aggregates only)')
    outside.set_defaults(func=cmd_outside)

    shape = sub.add_parser('shape')
    shape.add_argument('trace')
    shape.add_argument('--square', type=int, default=10,
                       help='half-width of the square to compare the edge with')
    shape.add_argument('--list', action='store_true',
                       help='print raw lists (default: aggregates only)')
    shape.set_defaults(func=cmd_shape)

    args = parser.parse_args(argv)
    rows = load(args.trace)
    args.func(Context(rows), args)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))