//! The chunk → region grid (ARCH-M4 §1, M4.3 part B).
//!
//! A region owns a square tile of chunks whose side is `region_side` chunks.
//! The parameter is a configuration value; the **default `0` means "no
//! split"**: the whole world is [`SINGLE_REGION`], exactly the pre-M4.3
//! behaviour, so the live server is unchanged until the owner raises the
//! value (the reason `0` and not a tile from the origin is recorded in
//! `docs/DECISIONS.md`).
//!
//! Chunk `(cx, cz)` belongs to tile `(cx / side, cz / side)` with a floored
//! (Euclidean) division, so negative coordinates fall on the correct side of
//! the origin. The tile is packed into one [`RegionId`] (`i32`); coordinates
//! whose tile index leaves `[-32768, 32767]` would wrap, a documented limit
//! that no M4.3 scenario reaches.

use super::region::{RegionId, SINGLE_REGION};

/// Blocks along one horizontal axis of a chunk: the game's fixed 16.
pub const CHUNK_SIZE: i32 = 16;

/// The chunk → region mapping of the world.
///
/// `Copy` and cheap: a region carries no reference to it, the tick receives
/// one by value (the pool sends it with the job).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RegionGrid {
    side: i32,
}

impl RegionGrid {
    /// The grid with the given tile side in chunks; `side <= 0` is "no
    /// split" ([`SINGLE_REGION`]).
    pub fn new(side: i32) -> Self {
        Self { side: side.max(0) }
    }

    /// The no-split grid: every chunk is [`SINGLE_REGION`].
    pub const fn single() -> Self {
        Self { side: 0 }
    }

    /// Tile side in chunks, `0` when the world is not split.
    pub fn side(&self) -> i32 {
        self.side
    }

    /// Whether the grid keeps the whole world in [`SINGLE_REGION`].
    pub fn is_single(&self) -> bool {
        self.side <= 0
    }

    /// Region of a chunk.
    pub fn region_of_chunk(&self, chunk_x: i32, chunk_z: i32) -> RegionId {
        if self.is_single() {
            return SINGLE_REGION;
        }
        let side = self.side;
        pack(chunk_x.div_euclid(side), chunk_z.div_euclid(side))
    }

    /// Region of a block.
    pub fn region_of_block(&self, x: i32, z: i32) -> RegionId {
        self.region_of_chunk(x.div_euclid(CHUNK_SIZE), z.div_euclid(CHUNK_SIZE))
    }

    /// Region of a precise player position (the chunk the position falls in).
    ///
    /// The join teleport and every applied move use this, so the region a
    /// player is transferred to is the region of the chunk it stands in.
    pub fn region_of_position(&self, x: f64, z: f64) -> RegionId {
        if self.is_single() {
            return SINGLE_REGION;
        }
        let chunk_x = floor_div(x, f64::from(CHUNK_SIZE));
        let chunk_z = floor_div(z, f64::from(CHUNK_SIZE));
        self.region_of_chunk(chunk_x, chunk_z)
    }
}

/// `floor(value / divisor)` as an `i32`, saturated to the `i32` range
/// (a non-finite coordinate falls back to the single region's chunk 0).
fn floor_div(value: f64, divisor: f64) -> i32 {
    let chunk = (value / divisor).floor();
    if !chunk.is_finite() {
        return 0;
    }
    chunk.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

/// Packs a tile index `(rx, rz)` into one `RegionId`, wrapping past 16 bits.
fn pack(rx: i32, rz: i32) -> RegionId {
    let high = (rx as u32 & 0xFFFF) << 16;
    let low = rz as u32 & 0xFFFF;
    (high | low) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_side_zero_puts_every_position_in_region_zero() {
        let grid = RegionGrid::new(0);
        assert!(grid.is_single());
        for (x, z) in [
            (0.0, 0.0),
            (0.5, 0.5),
            (-1.0, -1.0),
            (-30.5, 12.25),
            (12345.5, -98765.25),
            (f64::MAX, f64::MIN),
        ] {
            assert_eq!(grid.region_of_position(x, z), SINGLE_REGION);
        }
        // Negative `region_side` behaves exactly like no split.
        assert_eq!(RegionGrid::new(-4), RegionGrid::single());
        assert_eq!(
            RegionGrid::new(-4).region_of_block(999, -999),
            SINGLE_REGION
        );
    }

    #[test]
    fn a_tile_of_two_chunks_splits_on_the_block_boundary() {
        let grid = RegionGrid::new(2);
        let origin = grid.region_of_chunk(0, 0);
        assert_eq!(origin, SINGLE_REGION, "the origin tile keeps id 0");

        // Chunks (0,0) and (1,1) are the origin tile; (2,0) is the next one.
        assert_eq!(grid.region_of_chunk(1, 1), origin);
        assert_eq!(
            grid.region_of_block(31, 31),
            origin,
            "last block of chunk 1"
        );
        assert_ne!(
            grid.region_of_block(32, 0),
            origin,
            "first block of chunk 2"
        );
        assert_eq!(grid.region_of_block(32, 0), grid.region_of_chunk(2, 0));

        // The precise position crossing: 31.999 is still chunk 1, 32.0 chunk 2.
        assert_eq!(grid.region_of_position(31.999, 15.0), origin);
        assert_ne!(grid.region_of_position(32.0, 15.0), origin);
    }

    #[test]
    fn negative_chunks_use_floored_division() {
        let grid = RegionGrid::new(2);
        // Block -1 is chunk -1, tile (-1,0), which must not be the origin.
        let negative = grid.region_of_block(-1, 0);
        assert_ne!(negative, RegionGrid::new(2).region_of_block(0, 0));
        assert_eq!(negative, grid.region_of_chunk(-1, 0));
        // Tile rx=-1 covers chunks -2 and -1 (blocks -32..-1): block -32 is
        // still that tile, block -33 (chunk -3) is the next one.
        assert_eq!(grid.region_of_block(-32, 0), negative);
        assert_ne!(grid.region_of_block(-33, 0), negative);
        assert_eq!(grid.region_of_position(-0.5, 0.5), negative);
    }

    #[test]
    fn distinct_tiles_get_distinct_ids() {
        let grid = RegionGrid::new(2);
        let ids = [
            grid.region_of_chunk(0, 0),
            grid.region_of_chunk(2, 0),
            grid.region_of_chunk(0, 2),
            grid.region_of_chunk(-2, 0),
            grid.region_of_chunk(0, -2),
            grid.region_of_chunk(2, 2),
        ];
        let mut unique = ids.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), ids.len(), "ids: {ids:?}");
    }
}
