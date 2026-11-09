#!/usr/bin/env python3
"""Regression tests for `analyze.py` (plain `python`, no pytest needed).

    python tools/trace-analyze/test_analyze.py

The important one is `forget_coords`: round 5 documented the payload as
"two big-endian i32" but the traces show the first word is z (the packed
`ChunkPos` long), which put the forgotten chunks outside the loaded set.
"""
import struct
import sys
import unittest

import analyze


def row(name, payload):
    return {'name': name, 'payload': payload, 'dir': 'clientbound'}


class DecodeTest(unittest.TestCase):
    def test_forget_is_packed_chunk_pos_z_first(self):
        # ChunkPos.asLong(x=-4, z=-8) written big-endian.
        packed = struct.pack('>Q', ((-4 & 0xFFFFFFFF) |
                                    ((-8 & 0xFFFFFFFF) << 32)) & 0xFFFFFFFFFFFFFFFF)
        self.assertEqual(row(analyze.FORGET, packed)['payload'].hex(),
                         'fffffff8fffffffc')
        x, z = analyze.forget_coords(row(analyze.FORGET, packed))
        self.assertEqual((x, z), (-4, -8))

    def test_chunk_coords_is_x_then_z(self):
        self.assertEqual(
            analyze.chunk_coords(row(analyze.CHUNK, struct.pack('>ii', 6, -2))),
            (6, -2))

    def test_square_edge(self):
        self.assertEqual(analyze.square_edge(10, 1, 0), 21)
        self.assertEqual(analyze.square_edge(10, 0, 1), 21)
        self.assertEqual(analyze.square_edge(10, 1, 1), 41)
        self.assertEqual(analyze.square_edge(10, 2, 0), 42)

    def test_row_widths_axis(self):
        cells = {(0, 0), (1, 0), (1, 1), (2, 1)}
        by_x = analyze.row_widths(cells, (0, 0))
        self.assertEqual(by_x, {0: [0], 1: [0, 1], 2: [1]})
        by_z = analyze.row_widths(cells, (0, 0), axis=1)
        self.assertEqual(by_z, {0: [0, 1], 1: [1, 2]})


if __name__ == '__main__':
    unittest.main()
