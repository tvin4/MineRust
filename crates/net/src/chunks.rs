//! The join chunk window and the ready-batch index behind it (M4.4 A/B).
//!
//! Two pieces live here. [`window`] is the pure, deterministic list of
//! chunk coordinates a player at one cache centre asks for — the shape
//! vanilla 26.3 was measured to send (ЧАСТЬ 0 of M4.4, `docs/ARCH-M4.md`
//! §4). From M4.4 B it applies the measured `K(R)` formula for the traced
//! view distances `6..=12` ([`corner_cut`]) and the historical `(2R+1)²`
//! square elsewhere. [`ReadyChunks`] is the immutable ready batch every
//! connection joins, carrying the `(chunk_x, chunk_z) -> payload position`
//! index so a join resolves its window in O(1) per coordinate, never by
//! scanning the whole batch.

use std::collections::{BTreeSet, HashMap};

use protocol::LevelChunkWithLight;

/// Lowest view distance whose window shape the ЧАСТЬ 0 traces pinned.
///
/// Inside `TRACED_MIN_RADIUS..=TRACED_MAX_RADIUS` the window is a square of
/// half-width `R+1` with the corners cut by the circle `dx²+dz² ≤ K(R)`
/// (see [`corner_cut`]); outside the range it stays the historical
/// `(2R+1)²` square. The `K(R)` formula is a **[hypothesis]**: it is
/// verified against the four measured radii 6/8/10/12 (see the tests) and
/// against no other (ЧАСТЬ 0 of M4.4, `docs/ARCH-M4.md` §4, `DECISIONS.md`).
const TRACED_MIN_RADIUS: i32 = 6;

/// Upper end of the traced view-distance range (inclusive).
const TRACED_MAX_RADIUS: i32 = 12;

/// Offset from the view distance to the window's half-width: the measured
/// square reaches `R + 1` chunks from the centre (ЧАСТЬ 0 of M4.4).
const WINDOW_HALF_OFFSET: i32 = 1;

/// The corner-cut constant of the measured window, `(√3)²`.
///
/// ЧАСТЬ 0 measured a square half-width of `h = R + 1` with the corners
/// cut by the circle `dx²+dz² ≤ floor((h + √3)²)`. Because
/// `(h + √3)² = h² + 3 + 2h·√3 = h² + 3 + √(12·h²)`, the floor splits into
/// `h² + 3 + floor(√(12·h²))`: [`corner_cut`] therefore needs only integer
/// arithmetic, and the radius never depends on floating-point rounding.
/// The value `3` is exactly `(√3)²`.
const WINDOW_CUT: i64 = 3;

/// `K(h) = floor((h + √3)²)`, the squared radius the corner circle cuts
/// with at half-width `h`, in integers only (`h ≥ 0`).
fn corner_cut(h: i64) -> i64 {
    h * h + WINDOW_CUT + isqrt(12 * h * h)
}

/// The largest `r` with `r·r ≤ n` (`n ≥ 0`): the integer square root of the
/// corner-cut identity. A bit-by-bit algorithm, no floating point.
fn isqrt(n: i64) -> i64 {
    debug_assert!(n >= 0, "isqrt of a negative value");
    let mut value = n as u64;
    let mut root = 0u64;
    let mut bit = 1u64 << 62;
    while bit > value {
        bit >>= 2;
    }
    while bit != 0 {
        if value >= root + bit {
            value -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    root as i64
}

/// The chunk coordinates the window holds around `(center_x, center_z)` at
/// view distance `radius`, in send order.
///
/// At a traced view distance (`TRACED_MIN_RADIUS..=TRACED_MAX_RADIUS`) the
/// shape is the measured square of half-width `R+1` with the corners cut by
/// the circle `dx²+dz² ≤ K(R)` ([`corner_cut`]); at every other radius it
/// is the plain `(2R+1)²` square. The order is the one the ЧАСТЬ 0 traces
/// show within every flush: growing squared distance from the centre, ties
/// by `(chunk_x, chunk_z)`. A negative radius reads as zero (the centre
/// chunk alone).
pub fn window(center_x: i32, center_z: i32, radius: i32) -> Vec<(i32, i32)> {
    let radius = radius.max(0);
    let traced = (TRACED_MIN_RADIUS..=TRACED_MAX_RADIUS).contains(&radius);
    let half = if traced {
        radius + WINDOW_HALF_OFFSET
    } else {
        radius
    };
    let cut = if traced {
        corner_cut(i64::from(half))
    } else {
        i64::MAX
    };
    let side = 2 * half + 1;
    let mut out = Vec::with_capacity((side * side) as usize);
    for dx in -half..=half {
        for dz in -half..=half {
            let squared = i64::from(dx) * i64::from(dx) + i64::from(dz) * i64::from(dz);
            if squared > cut {
                continue;
            }
            out.push((center_x + dx, center_z + dz));
        }
    }
    out.sort_unstable_by_key(|&(x, z)| {
        let dx = i64::from(x - center_x);
        let dz = i64::from(z - center_z);
        (dx * dx + dz * dz, x, z)
    });
    out
}

/// A window shift (M4.4 B): the chunks entering the window and the chunks
/// leaving it, as [`window_difference`] returns them.
pub type WindowShift = (Vec<(i32, i32)>, Vec<(i32, i32)>);

/// The window change when the cache centre moves from `old` to `new` at view
/// distance `radius`, as a streaming event needs it (M4.4 B, ШАГ 2).
///
/// Returns `(add, remove)`:
///
/// * `add` = `window(new) − window(old)`, in the new centre's send order
///   ([`window`]) — the chunks the connection has to request;
/// * `remove` = `window(old) − window(new)`, ascending `(x, z)` — the traced
///   vanilla forget order (`window-p4/p5.jsonl`, ШАГ 1).
///
/// Both are pure and deterministic; the caller owns the centre update.
pub fn window_difference(old: (i32, i32), new: (i32, i32), radius: i32) -> WindowShift {
    let old_window = window(old.0, old.1, radius);
    let old_set: BTreeSet<(i32, i32)> = old_window.iter().copied().collect();
    let new_window = window(new.0, new.1, radius);
    let new_set: BTreeSet<(i32, i32)> = new_window.iter().copied().collect();
    // `BTreeSet::difference` walks the receiver in ascending `(x, z)`, which
    // is exactly the traced forget order.
    let remove: Vec<(i32, i32)> = old_set.difference(&new_set).copied().collect();
    // `add` keeps the new centre's send order, so it is a filtered
    // `new_window`, never a re-sort.
    let add: Vec<(i32, i32)> = new_window
        .into_iter()
        .filter(|cell| !old_set.contains(cell))
        .collect();
    (add, remove)
}

/// The immutable ready world behind a [`crate::loader::ChunkSource`]: the
/// wire payloads of every chunk the world can send and the index from
/// chunk coordinates to their position in that batch.
///
/// The index is built once, when the batch is built (the warm-up), and is
/// never written again: a join only reads it. There is no lock on any path
/// — the whole value sits behind one [`std::sync::Arc`].
pub struct ReadyChunks {
    /// Wire payloads, in the source's build order.
    chunks: Vec<LevelChunkWithLight>,
    /// `(chunk_x, chunk_z)` → position in `chunks`. A duplicate coordinate
    /// would overwrite an earlier one, which no world this server reads
    /// produces.
    index: HashMap<(i32, i32), usize>,
}

impl ReadyChunks {
    /// Builds the coordinate index over `chunks` once.
    pub fn new(chunks: Vec<LevelChunkWithLight>) -> Self {
        let index = chunks
            .iter()
            .enumerate()
            .map(|(position, chunk)| ((chunk.chunk_x, chunk.chunk_z), position))
            .collect();
        Self { chunks, index }
    }

    /// Every ready payload, in build order.
    pub fn chunks(&self) -> &[LevelChunkWithLight] {
        &self.chunks
    }

    /// How many chunks are ready to send.
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// Whether the world can send nothing at all.
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// The position of the chunk at `(chunk_x, chunk_z)`, or `None` when
    /// the world does not hold it — the O(1) lookup a join filters its
    /// window with.
    pub fn position(&self, chunk_x: i32, chunk_z: i32) -> Option<usize> {
        self.index.get(&(chunk_x, chunk_z)).copied()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    /// The window as a set, for the shape assertions.
    fn set(center_x: i32, center_z: i32, radius: i32) -> BTreeSet<(i32, i32)> {
        window(center_x, center_z, radius).into_iter().collect()
    }

    /// How many chunks sit in each `x - center_x = 0, 1, 2, …` column — the
    /// per-|dx| row width `analyze.py shape-join` prints (one signed side of
    /// the symmetric window).
    fn width_by_delta(cells: &BTreeSet<(i32, i32)>, center_x: i32) -> Vec<usize> {
        let widest = cells
            .iter()
            .map(|(x, _)| (x - center_x).abs())
            .max()
            .unwrap_or(0);
        (0..=widest)
            .map(|delta| cells.iter().filter(|(x, _)| x - center_x == delta).count())
            .collect()
    }

    /// A minimal well-formed payload for the index tests.
    fn test_chunk(chunk_x: i32, chunk_z: i32) -> LevelChunkWithLight {
        LevelChunkWithLight {
            chunk_x,
            chunk_z,
            heightmaps: Vec::new(),
            sections: Vec::new(),
            block_entities: Vec::new(),
            light: protocol::LightData {
                sky_mask: Vec::new(),
                block_mask: Vec::new(),
                empty_sky_mask: Vec::new(),
                empty_block_mask: Vec::new(),
                sky_arrays: Vec::new(),
                block_arrays: Vec::new(),
            },
        }
    }

    /// The measured control: R=10 is 473 chunks, one per traced row width.
    #[test]
    fn the_traced_window_holds_the_measured_473() {
        let cells = window(6, -2, 10);
        assert_eq!(cells.len(), 473, "the traced R=10 window");
        let distinct: BTreeSet<(i32, i32)> = cells.iter().copied().collect();
        assert_eq!(distinct.len(), 473, "no coordinate is repeated");
        assert_eq!(
            width_by_delta(&distinct, 6),
            vec![23, 23, 23, 23, 23, 23, 23, 21, 19, 19, 15, 13],
            "per-|dx| rows 0..6 → 23, then 21, 19, 19, 15, 13"
        );
    }

    /// The traced set is symmetric about its centre on both axes and under
    /// a half turn — the property the ЧАСТЬ 0 traces showed at every radius.
    #[test]
    fn the_traced_window_is_symmetric_about_its_centre() {
        let cells = set(6, -2, 10);
        for &(x, z) in &cells {
            assert!(cells.contains(&(12 - x, z)), "mirror x of ({x}, {z})");
            assert!(cells.contains(&(x, -4 - z)), "mirror z of ({x}, {z})");
            assert!(cells.contains(&(12 - x, -4 - z)), "180° of ({x}, {z})");
        }
    }

    /// A one-chunk shift adds exactly the 23-cell leading edge and drops
    /// the mirror edge — the П4 measurement, on both axes.
    #[test]
    fn a_one_chunk_shift_adds_the_leading_edge() {
        let here = set(6, -2, 10);
        let east = set(7, -2, 10);
        assert_eq!(east.difference(&here).count(), 23, "the +x shift adds 23");
        assert_eq!(
            here.difference(&east).count(),
            23,
            "and forgets the mirror 23"
        );
        let north = set(6, -1, 10);
        assert_eq!(north.difference(&here).count(), 23, "the +z shift adds 23");
        assert_eq!(here.difference(&north).count(), 23, "and forgets 23");
    }

    /// The diagonal shift adds the two leading edges once: 23 + 23 − 9 =
    /// 37 (the П5 measurement).
    #[test]
    fn a_diagonal_shift_adds_the_two_leading_edges_once() {
        let here = set(6, -2, 10);
        let diagonal = set(7, -1, 10);
        assert_eq!(diagonal.difference(&here).count(), 37, "+x/+z added");
        assert_eq!(here.difference(&diagonal).count(), 37, "the far corner");
    }

    /// [`window_difference`] splits a shift into `add` (new centre's send
    /// order, so it is a filtered `window(new)`) and `remove` (ascending
    /// `(x, z)`, the traced forget order) — the two orders the streaming
    /// event and the forget codec carry.
    #[test]
    fn the_window_difference_is_ordered_like_the_traces() {
        let old = (6, -2);
        let new = (7, -1);
        let (add, remove) = window_difference(old, new, 10);
        assert_eq!(add.len(), 37, "the diagonal adds its two edges once");
        assert_eq!(remove.len(), 37, "and removes the mirror corner");

        let old_set = set(old.0, old.1, 10);
        let new_set = set(new.0, new.1, 10);
        let expected_add: Vec<(i32, i32)> = window(new.0, new.1, 10)
            .into_iter()
            .filter(|cell| !old_set.contains(cell))
            .collect();
        assert_eq!(add, expected_add, "add keeps window(new) send order");
        let expected_remove: Vec<(i32, i32)> = old_set.difference(&new_set).copied().collect();
        assert_eq!(remove, expected_remove, "remove is ascending (x, z)");
        assert!(remove.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    /// The measured radii reproduce their traced counts exactly: the `K(R)`
    /// formula of ЧАСТЬ 0, in integers, at every point the trace pinned.
    #[test]
    fn the_formula_reproduces_every_measured_radius() {
        for (radius, expected) in [(6, 213), (8, 329), (10, 473), (12, 637)] {
            let cells = window(6, -2, radius);
            assert_eq!(cells.len(), expected, "R={radius}");
            let distinct: BTreeSet<(i32, i32)> = cells.iter().copied().collect();
            assert_eq!(distinct.len(), expected, "R={radius} is a set");
        }
    }

    /// [`corner_cut`] equals the four circles the ЧАСТЬ 0 traces measured:
    /// each radius' `K` interval (`docs/ARCH-M4.md` §4) contains exactly
    /// this value.
    #[test]
    fn the_corner_cut_matches_the_measured_circles() {
        assert_eq!(corner_cut(7), 76, "R=6 half-width 7");
        assert_eq!(corner_cut(9), 115, "R=8 half-width 9");
        assert_eq!(corner_cut(11), 162, "R=10 half-width 11");
        assert_eq!(corner_cut(13), 217, "R=12 half-width 13");
    }

    /// The integer square root is the floor of the real one, perfect squares
    /// included — the corner-cut identity depends on it.
    #[test]
    fn the_integer_square_root_floors_the_real_one() {
        let cases = [
            (0, 0),
            (1, 1),
            (3, 1),
            (4, 2),
            (588, 24),
            (972, 31),
            (1452, 38),
            (2028, 45),
        ];
        for (n, expected) in cases {
            assert_eq!(isqrt(n), expected, "isqrt({n})");
        }
    }

    /// Every radius outside the traced range is the plain `(2R+1)²` square —
    /// the shape the code keeps where the trace established no formula.
    #[test]
    fn every_untraced_radius_is_the_full_square() {
        for radius in [0, 1, 2, 5, 13, 16] {
            let cells = window(3, 4, radius);
            let side = (2 * radius + 1) as usize;
            assert_eq!(cells.len(), side * side, "radius {radius}");
            let distinct: BTreeSet<(i32, i32)> = cells.iter().copied().collect();
            assert_eq!(distinct.len(), side * side, "radius {radius} is a set");
        }
        assert_eq!(window(0, 0, 0), vec![(0, 0)], "radius 0 is the centre");
        assert_eq!(window(0, 0, -3).len(), 1, "a negative radius reads as 0");
    }

    /// The send order: growing squared distance, ties by coordinates.
    #[test]
    fn the_window_is_ordered_by_distance_then_coordinates() {
        let cells = window(6, -2, 10);
        let key = |&(x, z): &(i32, i32)| {
            let dx = i64::from(x - 6);
            let dz = i64::from(z + 2);
            (dx * dx + dz * dz, x, z)
        };
        assert_eq!(cells[0], (6, -2), "the centre first");
        assert!(
            cells.windows(2).all(|pair| key(&pair[0]) < key(&pair[1])),
            "the key strictly grows"
        );
    }

    /// The index finds every chunk that is in the batch and nothing else.
    #[test]
    fn the_ready_index_finds_every_chunk_and_nothing_else() {
        let chunks = vec![test_chunk(0, 0), test_chunk(1, 0), test_chunk(2, -2)];
        let ready = ReadyChunks::new(chunks);
        assert_eq!(ready.len(), 3);
        assert!(!ready.is_empty());
        assert_eq!(ready.position(0, 0), Some(0));
        assert_eq!(ready.position(1, 0), Some(1));
        assert_eq!(ready.position(2, -2), Some(2));
        assert_eq!(ready.position(0, 1), None, "a chunk the world lacks");
        assert_eq!(ready.position(-1, -1), None);
        assert_eq!(ready.chunks().len(), 3);
    }
}
