//! Uniform grid for neighbour queries.
//!
//! The Social Force Model asks "which agents are within a couple of metres of
//! this one" for every agent, every tick. At 25,000 agents and 20 Hz that is
//! half a million queries a second, and it is the dominant cost in the step
//! loop (`docs/04-track-b-simulation-engine.md` §B2 budgets 4 ms of an 8 ms
//! tick to it).
//!
//! # Why rebuild every tick instead of updating incrementally
//!
//! Rebuilding is O(n) by counting sort and touches memory linearly. Incremental
//! maintenance means a scattered read-modify-write per moved agent, and nearly
//! every agent moves every tick. Rebuilding is also trivially parallel and has
//! no ordering hazards, whereas incremental updates would need care to stay
//! deterministic.
//!
//! # The layout that matters
//!
//! The sort produces agent indices **grouped by cell and ascending within a
//! cell**. Iterating neighbours then walks contiguous runs of that array rather
//! than chasing pointers, which is what makes the force pass cache-friendly.
//! Ascending order within a cell also makes the neighbour list independent of
//! insertion order — a determinism requirement, not a nicety.

use cf_geom::{Aabb, Vec2};

/// A uniform grid over agent positions, rebuilt each tick.
#[derive(Clone, Debug, Default)]
pub struct SpatialGrid {
    origin: Vec2,
    cell_size: f64,
    inv_cell: f64,
    cols: usize,
    rows: usize,
    /// Start offset of each cell's run in [`items`], length `cols * rows + 1`.
    starts: Vec<u32>,
    /// Agent indices, grouped by cell and ascending within each cell.
    items: Vec<u32>,
    /// Scratch, retained across rebuilds to avoid reallocating.
    counts: Vec<u32>,
}

impl SpatialGrid {
    /// Cell size should be at least the largest interaction radius, so a query
    /// never needs to look beyond the 3x3 neighbourhood.
    pub fn new(bounds: Aabb, cell_size: f64) -> Self {
        let cell_size = cell_size.max(1e-3);
        // One cell of margin so agents exactly on the boundary still land in
        // the grid rather than being clamped into a neighbour's cell.
        let origin = Vec2::new(bounds.min.x - cell_size, bounds.min.y - cell_size);
        let cols = ((bounds.width() / cell_size).ceil() as usize + 3).max(1);
        let rows = ((bounds.height() / cell_size).ceil() as usize + 3).max(1);

        Self {
            origin,
            cell_size,
            inv_cell: 1.0 / cell_size,
            cols,
            rows,
            starts: vec![0; cols * rows + 1],
            items: Vec::new(),
            counts: vec![0; cols * rows],
        }
    }

    pub fn cell_size(&self) -> f64 {
        self.cell_size
    }

    pub fn dims(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Grid coordinates of a world position, clamped to the grid.
    #[inline]
    fn coords(&self, p: Vec2) -> (usize, usize) {
        let cx = ((p.x - self.origin.x) * self.inv_cell).floor();
        let cy = ((p.y - self.origin.y) * self.inv_cell).floor();
        let cx = (cx.max(0.0) as usize).min(self.cols - 1);
        let cy = (cy.max(0.0) as usize).min(self.rows - 1);
        (cx, cy)
    }

    #[inline]
    fn cell_index(&self, p: Vec2) -> usize {
        let (cx, cy) = self.coords(p);
        cy * self.cols + cx
    }

    /// Rebuild from scratch by counting sort.
    ///
    /// `active` selects which agents participate — agents that have exited
    /// should not repel the ones still inside.
    pub fn rebuild(&mut self, positions_x: &[f32], positions_y: &[f32], active: &[bool]) {
        debug_assert_eq!(positions_x.len(), positions_y.len());
        debug_assert_eq!(positions_x.len(), active.len());

        let n_cells = self.cols * self.rows;
        self.counts.clear();
        self.counts.resize(n_cells, 0);

        // Pass 1: count per cell.
        for i in 0..positions_x.len() {
            if !active[i] {
                continue;
            }
            let c = self.cell_index(Vec2::new(positions_x[i] as f64, positions_y[i] as f64));
            self.counts[c] += 1;
        }

        // Pass 2: prefix sum into cell start offsets.
        self.starts.clear();
        self.starts.resize(n_cells + 1, 0);
        let mut acc = 0u32;
        for c in 0..n_cells {
            self.starts[c] = acc;
            acc += self.counts[c];
        }
        self.starts[n_cells] = acc;

        // Pass 3: scatter. Walking agents in ascending index order means each
        // cell's run comes out ascending too, so neighbour lists do not depend
        // on insertion order.
        self.items.clear();
        self.items.resize(acc as usize, 0);
        let mut cursor = self.starts.clone();
        for i in 0..positions_x.len() {
            if !active[i] {
                continue;
            }
            let c = self.cell_index(Vec2::new(positions_x[i] as f64, positions_y[i] as f64));
            self.items[cursor[c] as usize] = i as u32;
            cursor[c] += 1;
        }
    }

    /// Agent indices in one cell.
    #[inline]
    fn cell_items(&self, cx: usize, cy: usize) -> &[u32] {
        let c = cy * self.cols + cx;
        let (s, e) = (self.starts[c] as usize, self.starts[c + 1] as usize);
        &self.items[s..e]
    }

    /// Visit every agent in the 3x3 neighbourhood around `p`.
    ///
    /// Callers must still check the actual distance: this returns everything in
    /// the surrounding cells, which is a superset of the true neighbours.
    /// Filtering here would cost a second distance computation, since the
    /// caller needs the delta vector anyway.
    #[inline]
    pub fn for_each_near(&self, p: Vec2, mut f: impl FnMut(u32)) {
        if self.items.is_empty() {
            return;
        }
        let (cx, cy) = self.coords(p);
        let x0 = cx.saturating_sub(1);
        let y0 = cy.saturating_sub(1);
        let x1 = (cx + 1).min(self.cols - 1);
        let y1 = (cy + 1).min(self.rows - 1);

        for y in y0..=y1 {
            for x in x0..=x1 {
                for &i in self.cell_items(x, y) {
                    f(i);
                }
            }
        }
    }

    /// Collect the 3x3 neighbourhood into a buffer. Convenience for tests and
    /// for code outside the hot loop; the loop itself uses
    /// [`SpatialGrid::for_each_near`] to avoid the allocation.
    pub fn near(&self, p: Vec2, out: &mut Vec<u32>) {
        out.clear();
        self.for_each_near(p, |i| out.push(i));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Aabb {
        Aabb {
            min: Vec2::new(0.0, 0.0),
            max: Vec2::new(20.0, 12.0),
        }
    }

    /// Build a grid from `(x, y)` pairs, all active.
    fn build(points: &[(f32, f32)], cell: f64) -> (SpatialGrid, Vec<f32>, Vec<f32>) {
        let xs: Vec<f32> = points.iter().map(|p| p.0).collect();
        let ys: Vec<f32> = points.iter().map(|p| p.1).collect();
        let active = vec![true; points.len()];
        let mut g = SpatialGrid::new(bounds(), cell);
        g.rebuild(&xs, &ys, &active);
        (g, xs, ys)
    }

    #[test]
    fn an_empty_grid_yields_nothing() {
        let (g, _, _) = build(&[], 2.0);
        assert!(g.is_empty());
        let mut out = Vec::new();
        g.near(Vec2::new(5.0, 5.0), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn every_agent_is_indexed_exactly_once() {
        let pts: Vec<(f32, f32)> = (0..500)
            .map(|i| ((i % 20) as f32, (i / 20) as f32 * 0.5))
            .collect();
        let (g, _, _) = build(&pts, 2.0);

        assert_eq!(g.len(), 500);
        let mut seen = vec![0u32; 500];
        for (cx, cy) in (0..g.dims().0).flat_map(|x| (0..g.dims().1).map(move |y| (x, y))) {
            for &i in g.cell_items(cx, cy) {
                seen[i as usize] += 1;
            }
        }
        assert!(
            seen.iter().all(|c| *c == 1),
            "some agent was indexed {:?} times",
            seen.iter().max()
        );
    }

    #[test]
    fn inactive_agents_are_excluded() {
        let xs = vec![1.0f32, 2.0, 3.0, 4.0];
        let ys = vec![1.0f32, 1.0, 1.0, 1.0];
        let active = vec![true, false, true, false];
        let mut g = SpatialGrid::new(bounds(), 2.0);
        g.rebuild(&xs, &ys, &active);

        assert_eq!(g.len(), 2, "only the active agents are indexed");
        let mut out = Vec::new();
        g.near(Vec2::new(2.5, 1.0), &mut out);
        assert!(out.contains(&0));
        assert!(out.contains(&2));
        assert!(!out.contains(&1));
        assert!(!out.contains(&3));
    }

    /// The property the force pass depends on: everything within one cell size
    /// must be returned. Missing a neighbour means a missing repulsion force,
    /// which shows up as agents quietly overlapping.
    #[test]
    fn all_true_neighbours_are_returned() {
        let mut seed = 0x5EEDu64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) as f64) / ((1u64 << 31) as f64)
        };

        let pts: Vec<(f32, f32)> = (0..400)
            .map(|_| ((next() * 20.0) as f32, (next() * 12.0) as f32))
            .collect();
        let cell = 2.0;
        let (g, xs, ys) = build(&pts, cell);

        let mut out = Vec::new();
        for i in 0..pts.len() {
            let p = Vec2::new(xs[i] as f64, ys[i] as f64);
            g.near(p, &mut out);

            // Brute force: everyone within one cell size.
            for j in 0..pts.len() {
                let q = Vec2::new(xs[j] as f64, ys[j] as f64);
                if p.distance(q) <= cell {
                    assert!(
                        out.contains(&(j as u32)),
                        "agent {j} at {q:?} is {:.3} m from {i} at {p:?} but was not returned",
                        p.distance(q)
                    );
                }
            }
        }
    }

    #[test]
    fn results_are_ascending_within_a_cell() {
        // All in one place, so they share a cell.
        let pts: Vec<(f32, f32)> = (0..50).map(|_| (5.0, 5.0)).collect();
        let (g, _, _) = build(&pts, 2.0);

        let mut out = Vec::new();
        g.near(Vec2::new(5.0, 5.0), &mut out);
        assert_eq!(out.len(), 50);

        let cell = g.cell_items(
            g.coords(Vec2::new(5.0, 5.0)).0,
            g.coords(Vec2::new(5.0, 5.0)).1,
        );
        let mut sorted = cell.to_vec();
        sorted.sort_unstable();
        assert_eq!(cell, &sorted[..], "cell contents must be ascending");
    }

    /// Rebuilding must be a pure function of its input, or the whole
    /// determinism guarantee fails at the first neighbour query.
    #[test]
    fn rebuilds_are_reproducible() {
        let pts: Vec<(f32, f32)> = (0..300)
            .map(|i| ((i % 17) as f32 * 1.1, (i % 11) as f32 * 1.05))
            .collect();
        let (a, _, _) = build(&pts, 2.0);
        let (b, _, _) = build(&pts, 2.0);
        assert_eq!(a.items, b.items);
        assert_eq!(a.starts, b.starts);

        // Reusing one grid across rebuilds must match a fresh one.
        let xs: Vec<f32> = pts.iter().map(|p| p.0).collect();
        let ys: Vec<f32> = pts.iter().map(|p| p.1).collect();
        let active = vec![true; pts.len()];
        let mut reused = SpatialGrid::new(bounds(), 2.0);
        reused.rebuild(&xs, &ys, &active);
        reused.rebuild(&xs, &ys, &active);
        assert_eq!(reused.items, a.items, "stale state leaked across rebuilds");
        assert_eq!(reused.starts, a.starts);
    }

    #[test]
    fn agents_outside_the_bounds_are_clamped_not_lost() {
        let xs = vec![-500.0f32, 500.0, 10.0];
        let ys = vec![-500.0f32, 500.0, 6.0];
        let active = vec![true; 3];
        let mut g = SpatialGrid::new(bounds(), 2.0);
        g.rebuild(&xs, &ys, &active);

        assert_eq!(g.len(), 3, "out-of-bounds agents must still be indexed");
        let mut out = Vec::new();
        g.near(Vec2::new(-500.0, -500.0), &mut out);
        assert!(out.contains(&0));
    }

    #[test]
    fn a_degenerate_cell_size_does_not_divide_by_zero() {
        let g = SpatialGrid::new(bounds(), 0.0);
        assert!(g.cell_size() > 0.0);
    }

    #[test]
    fn a_zero_area_bounds_still_works() {
        let b = Aabb {
            min: Vec2::new(3.0, 3.0),
            max: Vec2::new(3.0, 3.0),
        };
        let mut g = SpatialGrid::new(b, 2.0);
        g.rebuild(&[3.0], &[3.0], &[true]);
        let mut out = Vec::new();
        g.near(Vec2::new(3.0, 3.0), &mut out);
        assert_eq!(out, vec![0]);
    }
}
