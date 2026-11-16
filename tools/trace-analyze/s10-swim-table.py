#!/usr/bin/env python3
"""S10 swimming/crawling decision table from the mcproxy traces (Шаг 14e).

Observation only. The vanilla server is a black box:

* the *inputs* are the recorded client packets (A's serverbound player_input
  and move_player_*), the clientbound teleports the server itself sent
  (player_position), and the block layout the runner wrote into
  ``s10-samples.json`` (the exact /fill//setblock commands);
* the *answer* is what the vanilla server told the observer B (B's
  clientbound ``set_entity_data`` for A's entity id: the flags byte and the
  pose value).

Nothing here is copied from Minecraft. The block->fluid rule is a labelled
hypothesis that the table then confirms or refutes sample by sample.

Output:
* a human table of every flags/pose transition B saw, with the A input and the
  block/fluid at A's feet and eye at that moment;
* a summary of the observed (feet, eye, sprint, sneak) -> (flags, pose) map;
* a JSON dataset ``s10-samples-dataset.json`` for the Rust pose oracle.

Packet field layouts are hypotheses from the trace bytes (see
tools/trace-analyze/analyze.py for the older, length-checked variants).
"""
import argparse
import base64
import glob
import json
import os
import re
import struct
import sys
from datetime import datetime

sys.stdout.reconfigure(encoding='utf-8', errors='replace')

# player_input flag bits (serverbound byte), the mod's own key path order.
P_FORWARD = 0x01
P_BACKWARD = 0x02
P_LEFT = 0x04
P_RIGHT = 0x08
P_JUMP = 0x10
P_SNEAK = 0x20
P_SPRINT = 0x40

# entity metadata flags byte (clientbound), from docs/SWIMMING.md / S5/S8.
M_CROUCH = 0x02
M_SPRINT = 0x08
M_SWIM = 0x10

POSE_NAMES = {0: 'STANDING', 3: 'SWIMMING', 5: 'CROUCHING'}

# Blocks whose getFluidState() is a water source even though the block id is
# not minecraft:water. Hypothesis (owner': waterlogged property and water
# plants count as water) - the table confirms or refutes it.
PLANT_WATER = {
    'minecraft:seagrass',
    'minecraft:tall_seagrass',
    'minecraft:kelp',
    'minecraft:kelp_plant',
    'minecraft:bubble_column',
}


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


def read_trace(path):
    rows = []
    with open(path, encoding='utf-8', errors='replace') as handle:
        for line in handle:
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            row['payload'] = base64.b64decode(row.get('b64') or '')
            rows.append(row)
    rows.sort(key=lambda r: r['seq'])
    return rows


def client_name(rows):
    for row in rows:
        if row['name'] == 'minecraft:hello':
            body = row['payload']
            length, pos = varint(body, 0)
            return body[pos:pos + length].decode('utf-8', 'replace')
    return None


def parse_build(cmd):
    parts = cmd.split()
    if not parts:
        return None
    if parts[0] == 'fill' and len(parts) >= 8:
        x1, y1, z1, x2, y2, z2 = (int(v) for v in parts[1:7])
        return ('fill', x1, y1, z1, x2, y2, z2, parts[7])
    if parts[0] == 'setblock' and len(parts) >= 5:
        return ('setblock', int(parts[1]), int(parts[2]), int(parts[3]), parts[4])
    return None


def build_model(commands):
    model = {}
    for cmd in commands:
        parsed = parse_build(cmd)
        if not parsed:
            continue
        if parsed[0] == 'fill':
            _, x1, y1, z1, x2, y2, z2, block = parsed
            for x in range(min(x1, x2), max(x1, x2) + 1):
                for y in range(min(y1, y2), max(y1, y2) + 1):
                    for z in range(min(z1, z2), max(z1, z2) + 1):
                        model[(x, y, z)] = block
        else:
            _, x, y, z, block = parsed
            model[(x, y, z)] = block
    return model


def fluid_top(block):
    """(fluid surface offset above the block base, label) or (None, label).

    Rule from the geometry sweep (2026-11-16, r3), confirmed against the
    observer mod's own `FluidState.getOwnHeight()` readings (s10-samples.json):
    a player is "in water" exactly when the feet are strictly below the water
    surface. A water source tops out at `(8 - level) / 9`; a waterlogged block
    and the five water-shaped plants report a *full source* (`8/9`), which is
    what the client's `getOwnHeight()` returns for them. A top slab or a
    stair block does not appear here at all: the feet stand on top of it, so
    the feet block is the air cell above and carries no fluid.
    """
    if block is None:
        return None, 'air'
    name = block.split('[')[0]
    props = block[block.find('[') + 1:block.find(']')] if '[' in block else ''
    if name == 'minecraft:water':
        level = 0
        if 'level=' in props:
            try:
                level = int(props.split('level=')[1].split(',')[0].split(']')[0])
            except ValueError:
                level = 0
        return (8 - level) / 9.0, 'water'
    if 'waterlogged=true' in block:
        return 8 / 9.0, 'waterlogged'
    if name in PLANT_WATER:
        return 8 / 9.0, 'plant'
    return None, name.replace('minecraft:', '')


def in_fluid_at(model, x, y, z):
    """(is_in_fluid, label) with y the continuous feet height."""
    bx, by, bz = int(x // 1), int(y // 1), int(z // 1)
    top, label = fluid_top(model.get((bx, by, bz)))
    if top is None:
        return False, label
    return ((y - by) < top), label


def fluid_at(model, x, y, z):
    """Block-level fluid presence (kept for the head/eye columns)."""
    return fluid_top(model.get((int(x), int(y), int(z))))


def parse_entity_data_entries(payload):
    """[(index, type, value, length)] of a set_entity_data payload."""
    eid, pos = varint(payload, 0)
    entries = []
    while pos < len(payload) and payload[pos] != 0xFF:
        index = payload[pos]
        pos += 1
        ptype, pos = varint(payload, pos)
        if ptype == 0:  # byte
            value = payload[pos]
            pos += 1
        elif ptype == 1:  # varint
            value, pos = varint(payload, pos)
        elif ptype in (2, 3):  # float
            value = struct.unpack_from('>f', payload, pos)[0]
            pos += 4
        elif ptype == 4:  # string
            length, pos = varint(payload, pos)
            value = payload[pos:pos + length].decode('utf-8', 'replace')
            pos += length
        elif ptype == 20:  # pose (0x14): a varint pose enum, per the S8 bytes
            value, pos = varint(payload, pos)
        else:
            return eid, entries  # unknown: stop here, keep what we have
        entries.append((index, ptype, value))
    return eid, entries


def parse_add_entity(payload):
    eid, pos = varint(payload, 0)
    raw = payload[pos:pos + 16]
    pos += 16
    etype, pos = varint(payload, pos)
    x, y, z = struct.unpack_from('>ddd', payload, pos)
    return eid, raw, etype, (x, y, z)


def uuid_bytes(text):
    return bytes.fromhex(text.replace('-', ''))


def conn_anchors(run):
    """Wall-clock ms of each connection's recording start, from mcproxy-err.log.

    `read_ms` is per-connection (zero at that connection's own recording
    start), so two clients' traces do not share a clock: Bob connects about
    forty seconds after Alice. The anchor lines give the wall clock, so B's
    events can be moved into A's frame (docs/PROGRESS.md, "Сведение часов A/B").
    """
    path = os.path.join(run, 'mcproxy-err.log')
    out = {}
    if not os.path.exists(path):
        return out
    text = open(path, encoding='utf-8', errors='replace').read()
    text = re.sub(r'\x1b\[[0-9;]*m', '', text)
    for line in text.splitlines():
        match = re.search(
            r'(\S+Z)\s+INFO\s+\S+:\s+recording\s+.*proxy-conn-(\d+)\.jsonl',
            line.strip())
        if match:
            stamp = datetime.fromisoformat(match.group(1).replace('Z', '+00:00'))
            out[int(match.group(2))] = stamp.timestamp() * 1000.0
    return out


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--run', required=True)
    parser.add_argument('--out', default=None)
    args = parser.parse_args()

    artifact_path = os.path.join(args.run, 's10-samples.json')
    # PowerShell's Set-Content writes a UTF-8 BOM; utf-8-sig tolerates both.
    with open(artifact_path, encoding='utf-8-sig') as handle:
        artifact = json.load(handle)

    commands = list(artifact.get('platform') or [])
    for geo in artifact.get('geometries') or []:
        commands.extend(geo.get('commands') or [])
    model = build_model(commands)
    print(f'model blocks: {len(model)}')

    traces = sorted(glob.glob(os.path.join(args.run, 'mcproxy', 'proxy-conn-*.jsonl')))
    streams = {}
    conn_of = {}
    for path in traces:
        num = int(re.search(r'proxy-conn-(\d+)\.jsonl', path).group(1))
        rows = read_trace(path)
        name = client_name(rows)
        if name:
            streams[name] = rows
            conn_of[name] = num
    print(f'traces: {[(k, len(v)) for k, v in streams.items()]}')
    nick_a = artifact.get('nickA')
    nick_b = artifact.get('nickB')
    if nick_b is None:
        # Older artifacts (run r3 and earlier) recorded only the subject name;
        # the observer is the other connected client.
        nick_b = next((name for name in streams if name != nick_a), None)
    if nick_a not in streams or nick_b not in streams:
        print('missing A or B stream; names seen:', list(streams))
        return 1
    a_rows = streams[nick_a]
    b_rows = streams[nick_b]
    uuid_a = uuid_bytes(artifact['uuidA'])

    # Move B's event times into A's clock. Without this every sample pairs A's
    # position at the wrong moment with B's flags (the first analysis mixed
    # Alice's G1 position with Bob's G3 flags).
    anchors = conn_anchors(args.run)
    offset = 0.0
    if conn_of.get(nick_a) in anchors and conn_of.get(nick_b) in anchors:
        offset = anchors[conn_of[nick_b]] - anchors[conn_of[nick_a]]
        print(f'clock offset (B -> A frame): {offset:.1f} ms')
    else:
        print('WARNING: no anchors in mcproxy-err.log; A/B times are not aligned')

    # A's entity id as B sees it. The uuid the mod printed in the artifact has
    # its first eight bytes byte-reversed relative to the wire bytes (the proxy
    # shows `10920508 d5d83eed 93d292f193afe7d7` for the artifact
    # `08059210-d8d5-ed3e-93d2-92f193afe7d7`), so compare both orders. The
    # fallback is the single player-type entity in the observer's trace: Bob's
    # own entity is never added to himself, so exactly Alice remains.
    a_eid = None
    player_candidates = []
    for row in b_rows:
        if row['dir'] == 'clientbound' and row['name'] == 'minecraft:add_entity':
            eid, raw, etype, pos = parse_add_entity(row['payload'])
            swapped = raw[4:8] + raw[0:4] + raw[8:]
            if raw == uuid_a or swapped == uuid_a:
                a_eid = eid
            # 159 is the player entity type in 26.3 (PLAYER_ENTITY_TYPE).
            if etype == 159:
                player_candidates.append(eid)
    if a_eid is None and len(player_candidates) == 1:
        a_eid = player_candidates[0]
        print('uuid match failed; using the single player-type entity as A')
    print(f'A entity id seen by B: {a_eid}')
    if a_eid is None:
        return 1

    # A's input timeline and position timeline. Two serverbound streams carry
    # the sprint state: `player_input` (the held-key bitmask) and
    # `player_command` (action 1 = start sprinting, 2 = stop sprinting — the r3
    # trace shows one at every sprint toggle). The client sends the stop command
    # before the next `player_input` bit change, so the input byte alone
    # over-predicts sprint; the later of the two streams wins. Sneak is carried
    # only by `player_input` (holding shift produced no `player_command`).
    inputs = []
    sprint_commands = []
    positions = []
    teleports = []
    for row in sorted(a_rows, key=lambda r: r['read_ms']):
        t = row['read_ms']
        if row['dir'] == 'serverbound':
            if row['name'] == 'minecraft:player_input' and row['payload']:
                inputs.append((t, row['payload'][0]))
            elif row['name'] == 'minecraft:player_command' and row['payload']:
                try:
                    _, pos = varint(row['payload'], 0)
                    action, _ = varint(row['payload'], pos)
                except IndexError:
                    continue
                if action in (1, 2):
                    sprint_commands.append((t, action == 1))
            elif row['name'] in ('minecraft:move_player_pos', 'minecraft:move_player_pos_rot') \
                    and len(row['payload']) >= 24:
                positions.append((t, struct.unpack_from('>ddd', row['payload'], 0)))
        elif row['dir'] == 'clientbound' and row['name'] == 'minecraft:player_position':
            try:
                _, pos = varint(row['payload'], 0)
                if len(row['payload']) >= pos + 24:
                    where = struct.unpack_from('>ddd', row['payload'], pos)
                    positions.append((t, where))
                    teleports.append((t, where))
            except IndexError:
                pass
    print(f'A input events: {len(inputs)}  sprint commands: {len(sprint_commands)}  '
          f'position events: {len(positions)}  teleports: {len(teleports)}')

    def last_event(seq, when):
        found = None
        for t, value in seq:
            if t <= when:
                found = (t, value)
            else:
                break
        return found

    def last_before(seq, when):
        event = last_event(seq, when)
        return None if event is None else event[1]

    def sprint_at(when):
        """Effective sprint: the `player_command` start/stop stream.

        The r3 trace shows the `player_input` sprint bit (`0x40`) is only the
        *key*: A held it for 300 ms while standing still and vanilla never set
        the flag, then released it while still moving and vanilla kept the flag
        until the `player_command` stop arrived. The `player_command` actions
        (1 = start, 2 = stop) bracket the observed `0x08`/`0x00` exactly, so
        they, not the input bit, are the sprint state. Before the first command
        the state is "not sprinting" (A was standing).
        """
        command_event = last_event(sprint_commands, when)
        return command_event[1] if command_event is not None else False

    # B's metadata transitions for A.
    transitions = []
    for row in sorted(b_rows, key=lambda r: r['read_ms']):
        if row['dir'] != 'clientbound' or row['name'] != 'minecraft:set_entity_data':
            continue
        eid, entries = parse_entity_data_entries(row['payload'])
        if eid != a_eid:
            continue
        for index, ptype, value in entries:
            transitions.append((row['read_ms'], index, ptype, value))

    # Reducer so the two records of one tick (flags, pose) collapse.
    def sample_at(when):
        pos = last_before(positions, when) or (0.0, 0.0, 0.0)
        flags = last_before(inputs, when) or 0
        bx, by, bz = int(pos[0] // 1), int(pos[1] // 1), int(pos[2] // 1)
        # Feet: continuous height against the fluid surface (the r3 refinement).
        feet = in_fluid_at(model, pos[0], pos[1], pos[2])
        # Head block and the standing/crouched eye heights as block-level checks;
        # the eye height really depends on the pose (an output), so report both
        # the head block and the standing eye column.
        head = fluid_at(model, bx, by + 1, bz)
        eye = fluid_at(model, bx, int((pos[1] + 1.62) // 1), bz)
        sprint = sprint_at(when)
        sneak = bool(flags & P_SNEAK)
        return pos, flags, feet, head, eye, sprint, sneak

    # Geometry labelling: each sample belongs to the nearest build whose stand
    # is within a few blocks of A. The tunnels walk away from their stand, so
    # the radius is generous (8 blocks on x/z) and anything farther is "outside".
    geoms = []
    for geo in artifact.get('geometries') or []:
        stand = geo.get('stand') or {}
        geoms.append((str(geo.get('name')), float(stand.get('x', 0.0)),
                      float(stand.get('y', 0.0)), float(stand.get('z', 0.0))))

    def label_geom(pos):
        best, best_d = 'outside', 1e9
        for name, gx, gy, gz in geoms:
            if abs(pos[1] - gy) > 4.0:
                continue
            d = ((pos[0] - gx) ** 2 + (pos[2] - gz) ** 2) ** 0.5
            if d < best_d:
                best, best_d = name, d
        return best if best_d <= 8.0 else 'outside'

    # Every geometry is entered by a server teleport onto its stand; the
    # clientbound `player_position` that lands within a block of the stand is
    # the entry. Before it A is still at the previous geometry, but the nearest
    # stand can already be the next one (the slabs walked to z=-26.3, 2.8 blocks
    # from the bubble-column stand), so those "pre-teleport" samples carry the
    # wrong label. They are flagged, not dropped.
    def match_times(gx, gy, gz):
        return [t for t, (x, y, z) in teleports
                if abs(x - gx) < 1.5 and abs(z - gz) < 1.5 and abs(y - gy) < 3.0]

    geom_entry = {name: (min(match_times(gx, gy, gz)) if match_times(gx, gy, gz) else None)
                  for name, gx, gy, gz in geoms}

    def post_teleport(geom, when):
        entry = geom_entry.get(geom)
        return entry is not None and when >= entry

    # The authoritative server output: the flags byte (index 0) and the pose
    # (index 6, serializer 20). Health (9), air (1) and the game-mode byte (16)
    # are noise for this question.
    print()
    print('== B flags/pose transitions with A state at that instant ==')
    print(f'{"t_ms":>10} {"field":<6} {"value":>5}  {"geom":<18} {"post":>4} '
          f'{"pos":<24} {"in":>2} {"snk":>3} {"spr":>3}  feet / head / eye1.62')
    rows_out = []
    state_flags = None
    state_pose = None
    for t, index, ptype, value in transitions:
        t_a = t + offset  # B event time in A's clock
        pos, flags, feet, head, eye, sprint, sneak = sample_at(t_a)
        if index == 0:
            state_flags = value
        elif index == 6:
            state_pose = value
        geom = label_geom(pos)
        if index in (0, 6):
            label = 'flags' if index == 0 else 'pose'
            pos_s = f'({pos[0]:.2f},{pos[1]:.2f},{pos[2]:.2f})'
            print(f'{t_a:>10.1f} {label:<6} {value:>5}  {geom:<18} '
                  f'{str(post_teleport(geom, t_a)):>4} {pos_s:<24} '
                  f'{flags:>2x} {str(sneak):>3} {str(sprint):>3}  '
                  f'{feet[1]:<12} {head[1]:<12} {eye[1]}')
        rows_out.append({
            't_ms': t_a, 'index': index, 'value': value,
            'pos': list(pos), 'input_flags': flags,
            'sprint': sprint, 'sneak': sneak,
            'feet_water': feet[0], 'feet_kind': feet[1],
            'head_water': head[0] is not None, 'head_kind': head[1],
            'eye_water': eye[0] is not None, 'eye_kind': eye[1],
            'geometry': geom, 'post_teleport': post_teleport(geom, t_a),
            'flags': state_flags, 'pose': state_pose,
        })

    # A 50 ms tick grid over the observed window: our rule's prediction from A's
    # position/inputs at each tick against the flags B had last received. This
    # is the sample-level metric; the rows above are the transition-level one.
    flags_timeline = sorted((t + offset, v) for (t, idx, _pt, v) in transitions if idx == 0)
    ticks = []
    if flags_timeline:
        step = 50.0
        t = flags_timeline[0][0]
        end = flags_timeline[-1][0]
        while t <= end:
            pos, flags, feet, head, eye, sprint, sneak = sample_at(t)
            geom = label_geom(pos)
            ticks.append({
                't_ms': t, 'geometry': geom,
                'post_teleport': post_teleport(geom, t),
                'sprint': sprint, 'sneak': sneak, 'feet_water': feet[0],
                'observed_flags': last_before(flags_timeline, t),
            })
            t += step

    # Summary map of the confirmed pairs, keyed by the input conditions that a
    # candidate rule can read: the fluid at feet, head and the standing eye,
    # plus the two input bits.
    print()
    print('== observed map: (feet, head, eye, sprint, sneak) -> (flags, pose) ==')
    buckets = {}
    for row in rows_out:
        if row['flags'] is None:
            continue
        key = (row['feet_water'], row['head_water'], row['eye_water'],
               row['sprint'], row['sneak'])
        buckets.setdefault(key, set()).add((row['flags'], row['pose']))
    for key in sorted(buckets):
        feet, head, eye, sprint, sneak = key
        values = sorted(buckets[key], key=lambda fp: (fp[0], -1 if fp[1] is None else fp[1]))
        rendered = ', '.join(
            f'flags=0x{f:02x}({"/".join(n for b, n in ((M_CROUCH, "crouch"), (M_SPRINT, "sprint"), (M_SWIM, "swim")) if f & b) or "none"}) '
            f'pose={p}:{POSE_NAMES.get(p, p)}'
            for f, p in values)
        print(f'feet={"W" if feet else "-"} head={"W" if head else "-"} '
              f'eye={"W" if eye else "-"} sprint={int(sprint)} sneak={int(sneak)}'
              f'  ->  {rendered}')

    out_path = args.out or os.path.join(args.run, 's10-dataset.json')
    with open(out_path, 'w', encoding='utf-8') as handle:
        json.dump({'rows': rows_out, 'ticks': ticks, 'aEid': a_eid}, handle, indent=1)
    print(f'\ndataset: {out_path} ({len(rows_out)} rows, {len(ticks)} ticks)')
    return 0


if __name__ == '__main__':
    sys.exit(main())
