//! The join chunk window and the ready-batch index behind it (M4.4 A).
//!
//! Two pieces live here. [`window`] is the pure, deterministic list of
//! chunk coordinates a player at one cache centre asks for — the shape
//! vanilla 26.3 was measured to send (ЧАСТЬ 0 of M4.4, `docs/ARCH-M4.md`
//! §4). [`ReadyChunks`] is the immutable ready batch every connection
//! joins, carrying the `(chunk_x, chunk_z) -> payload position` index so a
//! join resolves its window in O(1) per coordinate, never by scanning the
//! whole batch.

use std::collections::HashMap;

use protocol::LevelChunkWithLight;

/// The one view distance whose window shape the ЧАСТЬ 0 trace established.
///
/// The shape is a square of half-width `R+1` with the corners cut by the
/// circle `dx²+dz² ≤ K`. `K` was measured per radius (74/113/162/208 for
/// R=6/8/10/12) and no formula `K(R)` follows from those four points, so
/// only R=10 — the control the walking traces already pinned — is wired
/// here; every other radius keeps the historical square (see the
/// `DECISIONS.md` entry, «гипотеза, не проверено по трейсу»).
const TRACED_RADIUS: i32 = 10;

/// Half-width of the traced window at [`TRACED_RADIUS`] (`R+1`).
const TRACED_HALF: i32 = 11;

/// Squared-circle cut of the traced window at [`TRACED_RADIUS`]: 473
/// chunks, `23×23 = 529` minus the 56 corner cells of the square.
const TRACED_MAX_SQUARED: i64 = 162;

/// The chunk coordinates the join window holds around `(center_x,
/// center_z)` at view distance `radius`, in send order.
///
/// The order is the one the ЧАСТЬ 0 traces show within every flush:
/// growing squared distance from the centre, ties by `(chunk_x, chunk_z)`.
/// A negative radius reads as zero (the centre chunk alone). The traced
/// circle applies only at [`TRACED_RADIUS`]; every other radius is the
/// full `(2R+1)×(2R+1)` square.
pub fn window(center_x: i32, center_z: i32, radius: i32) -> Vec<(i32, i32)> {
    let radius = radius.max(0);
    let traced = radius == TRACED_RADIUS;
    let half = if traced { TRACED_HALF } else { radius };
    let side = 2 * half + 1;
    let mut out = Vec::with_capacity((side * side) as usize);
    for dx in -half..=half {
        for dz in -half..=half {
            if traced {
                let squared = i64::from(dx) * i64::from(dx) + i64::from(dz) * i64::from(dz);
                if squared > TRACED_MAX_SQUARED {
                    continue;
                }
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

    /// Every radius but R=10 is the plain `(2R+1)²` square — the shape the
    /// code keeps where the trace did not establish a formula.
    #[test]
    fn every_other_radius_is_the_full_square() {
        for radius in [0, 1, 6, 8, 9, 11, 12, 16] {
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
