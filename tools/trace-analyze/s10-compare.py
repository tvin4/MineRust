#!/usr/bin/env python3
"""S10 vanilla vs our server: compare what observer B saw about A.

Both run artifacts (``s10-samples.json``) come from the same runner code, so
the rows align by ``geometry`` + ``phase`` + sample ordinal. For every sample
we rebuild the metadata flags byte B's client decoded
(``isCrouching`` = 0x02, ``isSprinting`` = 0x08, ``isSwimming`` = 0x10) and the
pose value, then diff vanilla against ours.

The output is the "geometry x action: vanilla vs ours" table the owner asked
for in Шаг 15 п.3, plus every mismatch with the sample index and the state A
reported (pose, feet/head/eye block ids, isInWater) so a mismatch can be read
as a geometry question, not a server bug.
"""
import argparse
import json
import sys
from collections import OrderedDict

sys.stdout.reconfigure(encoding='utf-8', errors='replace')

# Same labels the observer mod reads from Entity metadata.
M_CROUCH = 0x02
M_SPRINT = 0x08
M_SWIM = 0x10


def load(path):
    with open(path, encoding='utf-8-sig') as handle:
        document = json.load(handle)
    return document


def flags_of(sample):
    b = sample.get('b') or {}
    value = 0
    if b.get('isCrouching'):
        value |= M_CROUCH
    if b.get('isSprinting'):
        value |= M_SPRINT
    if b.get('isSwimming'):
        value |= M_SWIM
    return value


def key_of(sample):
    return sample.get('geometry'), sample.get('phase')


def align(vanilla, ours):
    """Pairs samples of the two runs by (geometry, phase, ordinal in phase)."""
    def index(document):
        counter = {}
        rows = OrderedDict()
        for sample in document['samples']:
            key = key_of(sample)
            ordinal = counter.get(key, 0)
            counter[key] = ordinal + 1
            rows.setdefault(key, []).append(sample)
        return rows

    left = index(vanilla)
    right = index(ours)
    pairs = []
    for key in left.keys() | right.keys():
        a = left.get(key, [])
        b = right.get(key, [])
        for ordinal in range(max(len(a), len(b))):
            pairs.append((key, ordinal, a[ordinal] if ordinal < len(a) else None,
                          b[ordinal] if ordinal < len(b) else None))
    return pairs


def block_id(sample, part):
    if not sample:
        return '?'
    block = (sample.get('a') or {}).get(part) or {}
    return block.get('id', '?')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--vanilla', required=True, help='vanilla run directory')
    parser.add_argument('--ours', required=True, help='our run directory')
    parser.add_argument('--limit', type=int, default=8,
                        help='mismatches printed per geometry (0 = all)')
    args = parser.parse_args()

    import os
    vanilla = load(os.path.join(args.vanilla, 's10-samples.json'))
    ours = load(os.path.join(args.ours, 's10-samples.json'))
    pairs = align(vanilla, ours)

    per_geometry = OrderedDict()
    total = 0
    same = 0
    for geometry in [g['name'] for g in vanilla['geometries']]:
        per_geometry[geometry] = {'samples': 0, 'same': 0, 'mismatch': [],
                                  'a_poses': set(), 'b_poses': set()}

    for (geometry, phase), ordinal, left, right in pairs:
        stats = per_geometry.setdefault(geometry, {'samples': 0, 'same': 0, 'mismatch': [],
                                                    'a_poses': set(), 'b_poses': set()})
        stats['samples'] += 1
        total += 1
        for sample in (left, right):
            if sample:
                stats['a_poses'].add((sample.get('a') or {}).get('pose'))
                stats['b_poses'].add((sample.get('b') or {}).get('pose'))
        if left is None or right is None:
            stats['mismatch'].append((phase, ordinal, 'missing sample', left, right))
            continue
        lf, rf = flags_of(left), flags_of(right)
        lp = (left.get('b') or {}).get('pose')
        rp = (right.get('b') or {}).get('pose')
        if lf == rf and lp == rp:
            stats['same'] += 1
            same += 1
        else:
            stats['mismatch'].append((phase, ordinal, f'flags {lf:#04x} vs {rf:#04x}, '
                                                      f'pose {lp} vs {rp}', left, right))

    print('S10 vanilla vs ours — observer B (flags = crouch 0x02 | sprint 0x08 | swim 0x10)')
    print()
    print(f"{'geometry':<20} {'samples':>7} {'same':>6} {'diff':>5}  "
          f"{'A poses':<34} {'B poses (vanilla)':<24}")
    for geometry, stats in per_geometry.items():
        a_poses = sorted(map(str, stats['a_poses']))
        b_poses = sorted(map(str, stats['b_poses']))
        print(f"{geometry:<20} {stats['samples']:>7} {stats['same']:>6} "
              f"{len(stats['mismatch']):>5}  {','.join(map(str, a_poses)):<34} "
              f"{','.join(map(str, b_poses)):<24}")

    print()
    print(f'total samples {total}, identical flags+pose {same}, '
          f'mismatches {total - same}')

    mismatched_geometries = [(g, s) for g, s in per_geometry.items() if s['mismatch']]
    if not mismatched_geometries:
        print('no flag or pose mismatch anywhere')
        return

    print()
    for geometry, stats in mismatched_geometries:
        print(f'== {geometry} — {len(stats["mismatch"])} mismatch(es)')
        limit = args.limit if args.limit > 0 else len(stats['mismatch'])
        for phase, ordinal, what, left, right in stats['mismatch'][:limit]:
            print(f'  {phase}[{ordinal}] {what}')
            for label, sample in (('vanilla', left), ('ours', right)):
                if sample is None:
                    print(f'    {label:<8} (no sample)')
                    continue
                a = sample.get('a') or {}
                b = sample.get('b') or {}
                print(f'    {label:<8} A pose={a.get("pose")} inWater={a.get("isInWater")} '
                      f'feet={block_id(sample, "feetBlock")} head={block_id(sample, "headBlock")} '
                      f'eye={block_id(sample, "eyeBlock")} | B pose={b.get("pose")} '
                      f'flags={flags_of(sample):#04x} swim={b.get("isSwimming")}')


if __name__ == '__main__':
    main()