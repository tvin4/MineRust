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
    python tools/trace-analyze/analyze.py entities <trace.jsonl>
                                                  --other NAME
                                                  [--self-trace <b.jsonl>]
                                                  [--list]
    python tools/trace-analyze/analyze.py entities-detail <trace.jsonl>
                                                  [--id 77]
                                                  [--other NAME]
                                                  [--self-trace <b.jsonl>]
                                                  [--gap 500] [--list]

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
* `entities` — the other player as one bot's trace shows it (M4.5
  two-bot trace): entity packet census, remove/add id lists, per-entity
  movement counts, and a `--self-trace` cross-check of the ids present
  in both bots' traces.
* `entities-detail` — the same trace filtered to **one** entity id (the
  other player): per-type counts, lengths and intervals about that id only,
  the mobs/info packets counted on separate lines and never analysed, the
  order of first occurrence, the leave/re-enter gaps, the decoded
  `add_entity` (uuid/type/position), and — with `--self-trace` — the
  player-to-player distance at the first and last packet about the id.
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
MOVE_ROT = 'minecraft:move_player_rot'
TICK_END = 'minecraft:client_tick_end'
BATCH_START = 'minecraft:chunk_batch_start'
BATCH_FINISHED = 'minecraft:chunk_batch_finished'
BATCH_RECEIVED = 'minecraft:chunk_batch_received'
CHUNK = 'minecraft:level_chunk_with_light'
CENTRE = 'minecraft:set_chunk_cache_center'
FORGET = 'minecraft:forget_level_chunk'
POSITION = 'minecraft:player_position'
# Entity-visibility packets of the two-bot trace (M4.5): the ids come from
# the data generator, the field layouts are decoded by length only.
ADD_ENTITY = 'minecraft:add_entity'
REMOVE_ENTITIES = 'minecraft:remove_entities'
PLAYER_INFO_UPDATE = 'minecraft:player_info_update'
PLAYER_INFO_REMOVE = 'minecraft:player_info_remove'
ENTITY_POS = 'minecraft:move_entity_pos'
ENTITY_POS_ROT = 'minecraft:move_entity_pos_rot'
ENTITY_ROT = 'minecraft:move_entity_rot'
ROTATE_HEAD = 'minecraft:rotate_head'
TELEPORT_ENTITY = 'minecraft:teleport_entity'
ENTITY_POSITION_SYNC = 'minecraft:entity_position_sync'


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


def chunk_flushes(rows):
    """The delivered chunk coordinates, grouped by batch interval."""
    flushes = []
    current = None
    for row in rows:
        if row['dir'] != 'clientbound':
            continue
        if row['name'] == BATCH_START:
            current = []
        elif row['name'] == CHUNK:
            coords = chunk_coords(row)
            if current is None:
                current = []
            if coords is not None:
                current.append(coords)
        elif row['name'] == BATCH_FINISHED:
            flushes.append(current or [])
            current = None
    if current:
        flushes.append(current)
    return flushes


def by_abs_axis(cells, centre, axis=0):
    """`|delta| -> width of that row` (one signed row; `+d` and `-d` agree
    on a symmetric window, the larger of the two is reported)."""
    out = {}
    for signed, values in row_widths(cells, centre, axis).items():
        key = abs(signed)
        out[key] = max(out.get(key, 0), len(values))
    return out


def smallest_matching_circle(cells, centre, half):
    """Smallest K whose circle in the half-width-`half` square equals cells."""
    for k in range(0, 2 * half * half + 1):
        circle = {(centre[0] + dx, centre[1] + dz)
                  for dx in range(-half, half + 1)
                  for dz in range(-half, half + 1)
                  if dx * dx + dz * dz <= k}
        if circle == cells:
            return k
    return None


def cmd_shape_join(ctx, args):
    """Join window of one trace: count, rows, half-width, matching circle,
    and the send order of the flushes (M4.4 part A, ЧАСТЬ 0)."""
    print(f'trace: {args.trace}')
    flushes = chunk_flushes(ctx.rows)
    cells = [coord for flush in flushes for coord in flush]
    distinct = set(cells)
    centres = [chunk_centre(r) for r in ctx.rows
               if r['dir'] == 'clientbound' and r['name'] == CENTRE]
    centres = [c for c in centres if c is not None]
    print(f'chunks: {len(cells)} delivered, {len(distinct)} distinct'
          f'  flushes: {len(flushes)}  first flush: {len(flushes[0]) if flushes else 0}')
    if not centres:
        print('no set_chunk_cache_center: cannot place the window')
        return
    centre = centres[-1]
    print(f'centre (last set_chunk_cache_center): {centre}'
          f'  centre moves: {len(centres)}')
    if not distinct:
        print('no chunks delivered')
        return
    half = max(max(abs(x - centre[0]), abs(z - centre[1])) for (x, z) in distinct)
    rows_x = by_abs_axis(distinct, centre, axis=0)
    rows_z = by_abs_axis(distinct, centre, axis=1)
    print(f'half-width h: {half}')
    print(f'  per-|dx| width: ' + ' '.join(f'{d}:{rows_x[d]}' for d in sorted(rows_x)))
    print(f'  per-|dz| width: ' + ' '.join(f'{d}:{rows_z[d]}' for d in sorted(rows_z)))
    full = (2 * half + 1) ** 2
    print(f'  a full (2h+1)^2 square of half-width {half} would be {full};'
          f' the window holds {len(distinct)} (missing {full - len(distinct)})')
    k = smallest_matching_circle(distinct, centre, half)
    if k is None:
        print('  circle |dx|2+|dz|2 <= K: no K inside half-width h matches the set')
        # The closest circle and what it cannot explain: the signal that the
        # world was clipped short of the window (edge rows come up short).
        best_k, diff = nearest_circle(distinct, centre, half)
        circle = {(centre[0] + dx, centre[1] + dz)
                  for dx in range(-half, half + 1)
                  for dz in range(-half, half + 1)
                  if dx * dx + dz * dz <= best_k}
        extra = sorted(cells_in(circle, distinct))
        missing = sorted(cells_in(distinct, circle))
        print(f'  closest circle K={best_k}: {diff} cells differ'
              f' (delivered {len(extra)} beyond, missing {len(missing)})')
        for coord in (extra + missing)[:12]:
            print(f'    off-circle cell {coord}')
    else:
        print(f'  circle |dx|2+|dz|2 <= {k} inside half-width {half} matches the set')
    order_report(flushes, centre)
    if args.list:
        for coord in sorted(distinct):
            print(f'  chunk {coord}')


def cells_in(left, right):
    """Cells of `left` that are not in `right`."""
    return set(left) - set(right)


def nearest_circle(cells, centre, half):
    """The circle of the half-width-`half` square closest to `cells`."""
    best_k, best_diff = 0, None
    for k in range(0, 2 * half * half + 1):
        circle = {(centre[0] + dx, centre[1] + dz)
                  for dx in range(-half, half + 1)
                  for dz in range(-half, half + 1)
                  if dx * dx + dz * dz <= k}
        diff = len(circle ^ cells)
        if best_diff is None or diff < best_diff:
            best_k, best_diff = k, diff
    return best_k, best_diff


def order_report(flushes, centre):
    """Whether the arrival order follows squared distance from the centre,
    globally and within every flush, with the number of regressions."""
    def sq(coord):
        return (coord[0] - centre[0]) ** 2 + (coord[1] - centre[1]) ** 2

    order = [coord for flush in flushes for coord in flush]
    global_reg = sum(1 for a, b in zip(order, order[1:]) if sq(a) > sq(b))
    flush_reg = 0
    for flush in flushes:
        flush_reg += sum(1 for a, b in zip(flush, flush[1:]) if sq(a) > sq(b))
    print(f'  send order: global distance regressions {global_reg};'
          f' within flushes {flush_reg} (0 means distance never falls)')
    if flushes:
        print(f'  first flush ({len(flushes[0])}): {flushes[0][:12]}')


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


ENTITY_KEYS = [
    ADD_ENTITY, REMOVE_ENTITIES, PLAYER_INFO_UPDATE, PLAYER_INFO_REMOVE,
    ENTITY_POS, ENTITY_POS_ROT, ENTITY_ROT, ROTATE_HEAD, TELEPORT_ENTITY,
    ENTITY_POSITION_SYNC,
    'minecraft:set_entity_data', 'minecraft:set_entity_motion',
    'minecraft:update_attributes', 'minecraft:set_equipment',
    'minecraft:entity_event', 'minecraft:set_passengers',
]

ENTITY_RE = re.compile(
    '|'.join(re.escape(n) for n in ENTITY_KEYS))


def add_entity_ids(row):
    """Entity ids inside an add_entity payload, decoded by length.

    The payload layout is not asserted here: the analyzer only extracts
    candidate ids the other bot's packets may reference. SOURCE: wiki,
    unverified — the id field position is a hypothesis checked by whether
    the two traces agree on it.
    """
    body = row['payload']
    # varint entity id + varint uuid-count... the exact prefix is a
    # hypothesis; read the first two varints and report them as candidates.
    try:
        eid, pos = varint(body, 0)
        _, pos2 = varint(body, pos)
        return [eid]
    except (IndexError, struct.error):
        return []


def remove_entity_ids(row):
    """Entity ids of remove_entities: varint count, then that many varints."""
    body = row['payload']
    try:
        count, pos = varint(body, 0)
        out = []
        for _ in range(min(count, 64)):
            eid, pos = varint(body, pos)
            out.append(eid)
        return out
    except (IndexError, struct.error):
        return []


def entity_varints(row):
    """First few varints of a move/rotate/teleport entity payload."""
    body = row['payload']
    out = []
    try:
        pos = 0
        for _ in range(2):
            v, pos = varint(body, pos)
            out.append(v)
    except (IndexError, struct.error):
        pass
    return out


def cmd_entities(ctx, args):
    """The other player as bot A's trace sees it: which entity packets
    reference B, in what order, with which lengths and intervals.

    Everything here is «определено по трейсу» except the field-layout
    remarks, which are hypotheses (marked inline). No raw coordinate or
    id list is printed without --list.
    """
    rows = ctx.rows
    other = args.other
    # Which entity id the other bot owns: cross-check add_entity against
    # the packets that move entities, then against bot B's own entity id
    # when --self-trace is given.
    print(f'trace: {args.trace}  other player: {other}')
    counts = Counter(r['name'] for r in rows
                     if r['dir'] == 'clientbound' and r['name'] in ENTITY_KEYS)
    print('entity packets (clientbound, by type):')
    for name, count in counts.most_common():
        print(f'  {name:<44} {count}')
    if not counts:
        print('  (none — the other player was never visible to this client)')

    # Linkage: add_entity is the only packet that introduces an entity;
    # its id is the hypothesis to test against the movement packets.
    add_rows = [r for r in rows if r['name'] == ADD_ENTITY
                and r['dir'] == 'clientbound']
    remove_rows = [r for r in rows if r['name'] == REMOVE_ENTITIES
                   and r['dir'] == 'clientbound']
    add_ids = []
    for r in add_rows:
        add_ids.extend(add_entity_ids(r))
    print(f'add_entity packets: {len(add_rows)}  remove_entities: '
          f'{len(remove_rows)}')
    if add_ids:
        print(f'entity ids seen in add_entity (layout hypothesis): '
              f'{sorted(set(add_ids))}')
    if remove_rows:
        removed = []
        for r in remove_rows:
            removed.extend(remove_entity_ids(r))
        print(f'entity ids seen in remove_entities: {sorted(set(removed))}')

    # Movement packet intervals and displacement per type.
    for name in (ENTITY_POS, ENTITY_POS_ROT, ENTITY_ROT, ROTATE_HEAD,
                 TELEPORT_ENTITY, ENTITY_POSITION_SYNC):
        sel = [r for r in rows
               if r['dir'] == 'clientbound' and r['name'] == name]
        if not sel:
            continue
        gaps = [b['t'] - a['t'] for a, b in zip(sel, sel[1:])]
        sizes = [len(r['payload']) for r in sel]
        print(f'{name}: n={len(sel)}  payload len min {min(sizes)} '
              f'median {statistics.median(sizes):.1f} max {max(sizes)}')
        if gaps:
            stat_line(f'  {name} interval (ms)', gaps)
    # Per-entity breakdown of the movement packets (the scene can hold
    # more entities than the other bot; the id prefix is the hypothesis).
    movers = [r for r in rows if r['dir'] == 'clientbound'
              and r['name'] in (ENTITY_POS, ENTITY_POS_ROT, TELEPORT_ENTITY)]
    per_entity = {}
    for r in movers:
        vs = entity_varints(r)
        if vs:
            per_entity.setdefault(vs[0], []).append(r)
    print('move_entity_* packets per entity id (id = first varint, '
          'layout hypothesis):')
    for eid, sel in sorted(per_entity.items()):
        print(f'  id {eid}: {len(sel)} packets')
    # Player-info packets (name/uuid of the other player), payload sizes.
    for name in (PLAYER_INFO_UPDATE, PLAYER_INFO_REMOVE):
        sel = [r for r in rows
               if r['dir'] == 'clientbound' and r['name'] == name]
        if sel:
            sizes = [len(r['payload']) for r in sel]
            print(f'{name}: n={len(sel)}  payload len min {min(sizes)} '
                  f'median {statistics.median(sizes):.1f} max {max(sizes)}')

    # The event sequence around the other player's join and leave: every
    # entity packet in order with its time and the bot's own position
    # (coordinates only with --list).
    if args.list:
        print('event order (clientbound entity packets, --list):')
        for r in rows:
            if r['dir'] == 'clientbound' and ENTITY_RE.fullmatch(r['name'] or ''):
                pos = nearest_position(rows, rows.index(r))
                where = (f'bot at ({pos[0]:.1f}, {pos[2]:.1f})'
                         if pos else 'bot position unknown')
                print(f'  t={r["t"]:.1f}  {r["name"]:<40} '
                      f'len={len(r["payload"]):<4} {where}')

    # Self-trace cross-check: does bot B's own trace carry the same
    # entity id that A's add_entity announced?
    if args.self_trace:
        self_rows = load(args.self_trace)
        own = [r for r in self_rows if r['name'] == ADD_ENTITY
               and r['dir'] == 'clientbound']
        own_ids = []
        for r in own:
            own_ids.extend(add_entity_ids(r))
        print(f'self trace {args.self_trace}: add_entity n={len(own)}'
              f'  ids (hypothesis) {sorted(set(own_ids))}')
        overlap = sorted(set(add_ids) & set(own_ids))
        print(f'ids in both traces (candidate linkage): {overlap}')


# The data generator's report (`reports/packets.json`) carries packet ids
# only, never field layouts, so every field position below is a length-pinned
# hypothesis (SOURCE: wiki, unverified) cross-checked against the other bot's
# own trace. The entity id on the first field is what `entities-detail`
# filters on; a wrong layout would show up as a missing/extra packet type
# about the id and as a distance that disagrees with the self-trace.

def uuid_str(raw):
    """Canonical 8-4-4-4-12 form of sixteen uuid bytes."""
    text = raw.hex()
    return (f'{text[0:8]}-{text[8:12]}-{text[12:16]}-'
            f'{text[16:20]}-{text[20:32]}')


def decode_add_entity(body):
    """(id, uuid, type, x, y, z, trailing) of an add_entity payload.

    Layout hypothesis: VarInt id, UUID(16 B), VarInt entity type,
    double x, y, z, ... (angles and metadata not decoded). `trailing` is how
    many bytes follow the position — a layout check, not a semantic value.
    """
    try:
        eid, pos = varint(body, 0)
        if len(body) < pos + 16:
            return None
        raw = body[pos:pos + 16]
        pos += 16
        etype, pos = varint(body, pos)
        if len(body) < pos + 24:
            return eid, raw, etype, None, None, None, len(body) - pos
        x, y, z = struct.unpack_from('>ddd', body, pos)
        return eid, raw, etype, x, y, z, len(body) - (pos + 24)
    except (IndexError, struct.error):
        return None


def decode_player_position(row):
    """(x, y, z) of the clientbound `player_position` (teleport id, then
    doubles). Layout hypothesis: VarInt teleport id, double x, y, z, ..."""
    body = row['payload']
    try:
        _, pos = varint(body, 0)
        if len(body) < pos + 24:
            return None
        return struct.unpack_from('>ddd', body, pos)
    except (IndexError, struct.error):
        return None


def entity_id_of(row):
    """The entity id a clientbound entity packet is about, or None.

    Hypotheses: the movement/data packets start with a VarInt entity id;
    `entity_event` starts with an event byte, the id follows. `remove_entities`
    (a list) and the `player_info_*` packets (no entity id) are handled by the
    caller, not here.
    """
    name = row['name']
    if name == ADD_ENTITY:
        decoded = decode_add_entity(row['payload'])
        return decoded[0] if decoded else None
    if name == 'minecraft:entity_event':
        try:
            eid, _ = varint(row['payload'], 1)
            return eid
        except (IndexError, struct.error):
            return None
    if name in (ENTITY_POS, ENTITY_POS_ROT, ENTITY_ROT, ROTATE_HEAD,
                TELEPORT_ENTITY, ENTITY_POSITION_SYNC,
                'minecraft:set_entity_data', 'minecraft:set_entity_motion',
                'minecraft:update_attributes', 'minecraft:set_equipment',
                'minecraft:set_passengers'):
        try:
            eid, _ = varint(row['payload'], 0)
            return eid
        except (IndexError, struct.error):
            return None
    return None


def cmd_entities_detail(ctx, args):
    """The trace filtered to one entity id (the other player).

    Mobs and every other entity are counted on a single line and never
    analysed; the `player_info_*` packets are reported separately (they name
    players, not entities). Everything printed is «определено по трейсу»
    except the field layouts, which are the marked hypotheses above.
    """
    rows = ctx.rows
    other_id = args.id
    info_update = [r for r in rows if r['dir'] == 'clientbound'
                   and r['name'] == PLAYER_INFO_UPDATE]
    info_remove = [r for r in rows if r['dir'] == 'clientbound'
                   and r['name'] == PLAYER_INFO_REMOVE]
    entity_rows = [r for r in rows if r['dir'] == 'clientbound'
                   and r['name'] in ENTITY_KEYS
                   and r['name'] not in (PLAYER_INFO_UPDATE, PLAYER_INFO_REMOVE)]
    about, mobs, unparsed = [], [], []
    for row in entity_rows:
        if row['name'] == REMOVE_ENTITIES:
            (about if other_id in remove_entity_ids(row) else mobs).append(row)
            continue
        eid = entity_id_of(row)
        if eid is None:
            unparsed.append(row)
        elif eid == other_id:
            about.append(row)
        else:
            mobs.append(row)
    about.sort(key=lambda r: r['seq'])
    mob_ids = set()
    for row in mobs:
        if row['name'] == REMOVE_ENTITIES:
            mob_ids.update(remove_entity_ids(row))
        else:
            eid = entity_id_of(row)
            if eid is not None:
                mob_ids.add(eid)
    print(f'trace: {args.trace}  other player: {args.other}'
          f'  other entity id: {other_id}')
    print(f'entity packets: total {len(entity_rows)}  about id {other_id}:'
          f' {len(about)}  other entities (mobs): {len(mobs)} (not analysed)'
          f'  unparsed: {len(unparsed)}')

    print(f'packets about id {other_id}, by type:')
    if not about:
        print('  (none — the other player was never visible to this client)')
    for name in ENTITY_KEYS:
        sel = [r for r in about if r['name'] == name]
        if not sel:
            continue
        sizes = [len(r['payload']) for r in sel]
        gaps = [b['t'] - a['t'] for a, b in zip(sel, sel[1:])]
        line = (f'  {name:<44} n={len(sel):>5}  len {min(sizes)}'
                f'/{statistics.median(sizes):.0f}/{max(sizes)}')
        if gaps:
            line += (f'  interval min {min(gaps):.1f} med'
                     f' {statistics.median(gaps):.1f} max {max(gaps):.1f} ms')
        print(line)

    print('packets about other entities (mobs), by type (counts only):')
    for name, count in Counter(r['name'] for r in mobs).most_common():
        print(f'  {name:<44} {count}')
    print(f'  distinct mob entity ids: {len(mob_ids)}  '
          f'unparsed (no id in layout): {len(unparsed)}')
    for name, count in Counter(r['name'] for r in unparsed).most_common():
        print(f'  unparsed {name}: {count}')

    order = list(dict.fromkeys(r['name'] for r in about))
    print(f'order of first occurrence about id {other_id}: '
          + (' -> '.join(order) if order else '(none)'))
    if about:
        first, last = about[0], about[-1]
        print(f'first: t={first["t"]:.1f} {first["name"]}  '
              f'last: t={last["t"]:.1f} {last["name"]}  '
              f'span {last["t"] - first["t"]:.1f} ms')
        gaps = [(b['t'], b['t'] - a['t']) for a, b in zip(about, about[1:])]
        big = sorted((g for g in gaps if g[1] >= args.gap),
                     key=lambda item: -item[1])
        print(f'gaps >= {args.gap:.0f} ms between packets about the id'
              f' (leave/re-enter): {len(big)}')
        for when, gap in big[:5]:
            print(f'  gap {gap:.0f} ms ending at t={when:.1f}')
    for row in about:
        if row['name'] != ADD_ENTITY:
            continue
        decoded = decode_add_entity(row['payload'])
        if not decoded:
            continue
        eid, raw, etype, x, y, z, trailing = decoded
        pos = 'unread' if x is None else f'({x:.3f}, {y:.3f}, {z:.3f})'
        print(f'add_entity id {eid}: uuid {uuid_str(raw)}  type {etype}'
              f'  pos {pos}  bytes after pos {trailing} (layout hypothesis)')
    if info_update or info_remove:
        print(f'player_info_update n={len(info_update)}  payload sizes'
              f' {[len(r["payload"]) for r in info_update]}')
        print(f'player_info_remove n={len(info_remove)}  payload sizes'
              f' {[len(r["payload"]) for r in info_remove]}')

    if args.self_trace:
        self_rows = load(args.self_trace)
        own_ids = []
        for row in self_rows:
            if row['dir'] == 'clientbound' and row['name'] == ADD_ENTITY:
                decoded = decode_add_entity(row['payload'])
                if decoded:
                    own_ids.append(decoded[0])
        print(f'self trace: add_entity {len(own_ids)}  ids seen'
              f' {sorted(set(own_ids))}')
        print(f'  id {other_id} present in the other bot\'s own adds:'
              f' {other_id in own_ids}')
        self_moves = [(r['t'], position(r)) for r in self_rows
                      if r['dir'] == 'serverbound'
                      and position(r) is not None]
        # This client (A) may stand still and send no moves: its own position
        # then comes from the single clientbound `player_position`.
        own_moves = []
        for row in rows:
            if row['dir'] == 'serverbound':
                coords = position(row)
            elif row['name'] == POSITION:
                coords = decode_player_position(row)
            else:
                continue
            if coords is not None:
                own_moves.append((row['t'], coords))
        own_moves.sort()
        self_moves.sort()

        def position_at(timeline, when):
            found = None
            for stamp, coords in timeline:
                if stamp <= when:
                    found = coords
                else:
                    break
            return found

        def distance(row):
            mine = position_at(own_moves, row['t'])
            theirs = position_at(self_moves, row['t'])
            if not mine or not theirs:
                return None
            dx = mine[0] - theirs[0]
            dz = mine[2] - theirs[2]
            return (dx * dx + dz * dz) ** 0.5

        for row in about:
            if row['name'] not in (ADD_ENTITY, REMOVE_ENTITIES):
                continue
            blocks = distance(row)
            shown = 'unread' if blocks is None else f'{blocks:.1f} blocks'
            print(f'{row["name"]} at t={row["t"]:.1f}: distance to this client '
                  f'{shown}')
        best = None
        for row in about:
            blocks = distance(row)
            if blocks is not None and (best is None or blocks > best[1]):
                best = (row['t'], blocks, row['name'])
        if best:
            print(f'max distance while visible: {best[1]:.1f} blocks'
                  f' (t={best[0]:.1f}, {best[2]})')

    if args.list:
        print(f'event order about id {other_id} (--list):')
        for row in about:
            eid = entity_id_of(row)
            detail = '' if eid is None else f' id={eid}'
            print(f'  t={row["t"]:>9.1f} {row["name"]:<44}'
                  f' len={len(row["payload"]):<4}{detail}')


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

    shape_join = sub.add_parser('shape-join')
    shape_join.add_argument('trace')
    shape_join.add_argument('--list', action='store_true',
                            help='print raw lists (default: aggregates only)')
    shape_join.set_defaults(func=cmd_shape_join)

    entities = sub.add_parser('entities')
    entities.add_argument('trace')
    entities.add_argument('--other', required=True,
                          help='name of the other player (label only)')
    entities.add_argument('--self-trace', default=None,
                          help='the other bot\u2019s own JSONL for the '
                               'entity-id cross-check')
    entities.add_argument('--list', action='store_true',
                          help='print the event order (default: aggregates '
                               'only)')
    entities.set_defaults(func=cmd_entities)

    entities_detail = sub.add_parser('entities-detail')
    entities_detail.add_argument('trace')
    entities_detail.add_argument('--id', type=int, default=77,
                                 help='entity id of the other player')
    entities_detail.add_argument('--other', default='other',
                                 help='name of the other player (label only)')
    entities_detail.add_argument('--self-trace', default=None,
                                 help='the other bot\u2019s own JSONL for the '
                                      'id cross-check and the distance')
    entities_detail.add_argument('--gap', type=float, default=500.0,
                                 help='gap in ms counted as a leave/re-enter')
    entities_detail.add_argument('--list', action='store_true',
                                 help='print the event order (default: '
                                      'aggregates only)')
    entities_detail.set_defaults(func=cmd_entities_detail)

    args = parser.parse_args(argv)
    rows = load(args.trace)
    args.func(Context(rows), args)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))