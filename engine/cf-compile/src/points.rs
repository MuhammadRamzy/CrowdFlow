//! Deduplicating point accumulator.
//!
//! `cf_navmesh::triangulate` rejects coincident points rather than dropping
//! them, because dropping would silently shift every downstream vertex index.
//! Deduplication is therefore the caller's job, and this is where it happens.
//!
//! Imported geometry routinely has near-coincident vertices: two walls meeting
//! at a corner each contribute an endpoint, and after scanning and vectorising
//! those endpoints differ in the last few decimal places. Snapping within
//! [`cf_geom::SNAP_EPSILON_M`] (1 mm — finer than architectural drawings are
//! dimensioned) merges them.

use cf_geom::{Vec2, SNAP_EPSILON_M};
use std::collections::HashMap;

/// Accumulates points, merging any that fall within the snap tolerance.
#[derive(Debug, Default)]
pub struct PointSet {
    points: Vec<Vec2>,
    /// Bucket index → point indices in that bucket. Bucket size is the snap
    /// tolerance, so a match can only be in the 3x3 neighbourhood.
    grid: HashMap<(i64, i64), Vec<usize>>,
    merged: usize,
}

impl PointSet {
    pub fn new() -> Self {
        Self::default()
    }

    fn cell(p: Vec2) -> (i64, i64) {
        (
            (p.x / SNAP_EPSILON_M).floor() as i64,
            (p.y / SNAP_EPSILON_M).floor() as i64,
        )
    }

    /// Insert a point, returning its index. Returns the existing index if an
    /// earlier point is within the snap tolerance.
    pub fn insert(&mut self, p: Vec2) -> usize {
        let (cx, cy) = Self::cell(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                if let Some(bucket) = self.grid.get(&(cx + dx, cy + dy)) {
                    for &i in bucket {
                        if self.points[i].distance(p) <= SNAP_EPSILON_M {
                            self.merged += 1;
                            return i;
                        }
                    }
                }
            }
        }
        let idx = self.points.len();
        self.points.push(p);
        self.grid.entry((cx, cy)).or_default().push(idx);
        idx
    }

    pub fn points(&self) -> &[Vec2] {
        &self.points
    }

    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// How many insertions were merged into an existing point.
    pub fn merged_count(&self) -> usize {
        self.merged
    }

    pub fn into_points(self) -> Vec<Vec2> {
        self.points
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_points_get_distinct_indices() {
        let mut s = PointSet::new();
        assert_eq!(s.insert(Vec2::new(0.0, 0.0)), 0);
        assert_eq!(s.insert(Vec2::new(10.0, 0.0)), 1);
        assert_eq!(s.insert(Vec2::new(10.0, 5.0)), 2);
        assert_eq!(s.len(), 3);
        assert_eq!(s.merged_count(), 0);
    }

    #[test]
    fn exact_duplicates_merge() {
        let mut s = PointSet::new();
        let a = s.insert(Vec2::new(3.0, 4.0));
        let b = s.insert(Vec2::new(3.0, 4.0));
        assert_eq!(a, b);
        assert_eq!(s.len(), 1);
        assert_eq!(s.merged_count(), 1);
    }

    /// The case that matters for imports: two walls meeting at a corner whose
    /// endpoints differ in the last decimal places.
    #[test]
    fn near_coincident_points_merge() {
        let mut s = PointSet::new();
        let a = s.insert(Vec2::new(10.0, 0.0));
        let b = s.insert(Vec2::new(10.0 + 1e-6, 1e-7));
        assert_eq!(a, b, "sub-millimetre difference should snap together");
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn points_further_than_the_tolerance_stay_separate() {
        let mut s = PointSet::new();
        let a = s.insert(Vec2::new(0.0, 0.0));
        // 2 mm apart — twice the tolerance.
        let b = s.insert(Vec2::new(0.002, 0.0));
        assert_ne!(a, b);
        assert_eq!(s.len(), 2);
    }

    /// A match may sit in an adjacent bucket, so the neighbourhood search
    /// matters. These two straddle a cell boundary.
    #[test]
    fn matches_across_a_bucket_boundary_are_found() {
        let mut s = PointSet::new();
        let a = s.insert(Vec2::new(SNAP_EPSILON_M * 3.0 - 1e-9, 0.0));
        let b = s.insert(Vec2::new(SNAP_EPSILON_M * 3.0 + 1e-9, 0.0));
        assert_eq!(a, b, "points straddling a bucket edge must still merge");
    }

    #[test]
    fn insertion_order_is_preserved() {
        let mut s = PointSet::new();
        s.insert(Vec2::new(5.0, 5.0));
        s.insert(Vec2::new(1.0, 1.0));
        s.insert(Vec2::new(5.0, 5.0));
        assert_eq!(
            s.points(),
            &[Vec2::new(5.0, 5.0), Vec2::new(1.0, 1.0)],
            "dedupe must not reorder"
        );
    }
}
