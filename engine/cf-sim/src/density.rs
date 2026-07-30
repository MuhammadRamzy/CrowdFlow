//! Crowd density measurement.
//!
//! Density is the headline safety figure. Crowd science treats 6 persons/m² as
//! the point where forward movement ceases and crush risk begins, so the number
//! this module produces is the one a safety officer actually acts on.
//!
//! # Why the counts are smoothed
//!
//! The obvious implementation drops each agent into the cell containing it and
//! divides by cell area. That is wrong in a way that matters: with 0.5 m cells
//! and ~0.46 m of personal space, a tight group straddling a cell boundary
//! splits across four cells and each reads a fraction of the true density. The
//! peak — the only value anyone cares about — is systematically under-reported,
//! and under-reporting a crush risk is the worst error this product can make.
//!
//! So each agent is spread over nearby cells with a normalised Gaussian kernel.
//! Total mass is preserved: summing density × cell area over the grid returns
//! the agent count exactly.
//!
//! # The kernel is a disc, not a Gaussian
//!
//! "Persons per square metre" has a literal definition: count the people within
//! a square metre and divide by one square metre. The kernel that implements
//! that definition is a **uniform disc of area 1 m²** (radius 1/√π ≈ 0.564 m),
//! softened at its rim so an agent crossing the boundary does not make the
//! reading jump.
//!
//! Two earlier attempts, both wrong, are worth recording because both looked
//! plausible:
//!
//! - **Gaussian, σ = 0.7 m.** Effective area ~3 m². Reported 9 people packed
//!   into one square metre as 2.5 p/m² — comfortably "safe" for a crowd that is
//!   in fact past the crush threshold.
//! - **Gaussian, σ = 1/√(2π) ≈ 0.4 m.** Effective area 1 m², so the *average*
//!   was right, but the profile is peaked: it resolves the crowd's own lattice
//!   instead of averaging over it. A perfectly packed lattice of 0.23 m bodies —
//!   5.46 p/m², the densest arrangement that physically exists — read as
//!   7.5 p/m². Reporting an impossible density is as damaging as missing a real
//!   one, because it makes every figure the tool produces suspect.
//!
//! The disc gets both cases right, because it is measuring the thing the number
//! is defined as rather than approximating it.
//!
//! # Peak is tracked per cell, not globally
//!
//! A single global maximum tells you a venue got dangerous somewhere. A per-cell
//! maximum tells you *where*, which is what changes a layout.

use crate::world::World;
use cf_geom::Aabb;

/// Crowd-science density thresholds, persons/m². These are the bands the
/// heatmap and the compliance engine both read.
pub const BAND_COMFORTABLE: f32 = 2.0;
pub const BAND_RESTRICTED: f32 = 4.0;
/// Forward movement ceases; crush risk. NFPA's jam-point density.
pub const BAND_CRITICAL: f32 = 6.0;

/// A density field over the venue.
#[derive(Clone, Debug)]
pub struct DensityGrid {
    origin_x: f64,
    origin_y: f64,
    cell: f64,
    cols: usize,
    rows: usize,
    /// Persons/m², current tick.
    current: Vec<f32>,
    /// Highest value each cell has reached, over the whole run.
    peak: Vec<f32>,
    /// Kernel weights, precomputed once.
    kernel: Vec<f32>,
    kernel_radius: i32,
}

impl DensityGrid {
    /// `cell` is the cell size in metres. 0.5 m is the design default: fine
    /// enough to resolve a doorway, coarse enough to stay cheap.
    pub fn new(bounds: Aabb, cell: f64) -> Self {
        let cell = cell.max(0.05);
        // A margin so agents at the very edge still contribute their full kernel.
        let pad = 2.0;
        let origin_x = bounds.min.x - pad;
        let origin_y = bounds.min.y - pad;
        let cols = (((bounds.width() + pad * 2.0) / cell).ceil() as usize).max(1);
        let rows = (((bounds.height() + pad * 2.0) / cell).ceil() as usize).max(1);

        // A disc of area 1 m². This is the measurement window, and its size is
        // what makes the output mean "persons per square metre".
        let disc_r = (1.0 / std::f64::consts::PI).sqrt();
        // Soften the rim over roughly one cell so an agent stepping across the
        // boundary does not make the reading jump.
        let soft = cell.min(0.15);
        let radius = ((disc_r + soft) / cell).ceil() as i32;

        let mut kernel = Vec::new();
        let mut sum = 0.0f64;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                let x = dx as f64 * cell;
                let y = dy as f64 * cell;
                let d = (x * x + y * y).sqrt();
                // 1 inside, 0 outside, smoothstep across the rim.
                let w = if d <= disc_r - soft {
                    1.0
                } else if d >= disc_r + soft {
                    0.0
                } else {
                    let t = (disc_r + soft - d) / (2.0 * soft);
                    t * t * (3.0 - 2.0 * t)
                };
                kernel.push(w as f32);
                sum += w;
            }
        }
        // Normalise so one agent contributes exactly one person of mass.
        if sum > 0.0 {
            for w in &mut kernel {
                *w = (*w as f64 / sum) as f32;
            }
        }

        Self {
            origin_x,
            origin_y,
            cell,
            cols,
            rows,
            current: vec![0.0; cols * rows],
            peak: vec![0.0; cols * rows],
            kernel,
            kernel_radius: radius,
        }
    }

    pub fn dims(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    pub fn cell_size(&self) -> f64 {
        self.cell
    }

    /// World-space origin of cell `(0, 0)`.
    pub fn origin(&self) -> (f64, f64) {
        (self.origin_x, self.origin_y)
    }

    /// Persons/m² this tick, row-major.
    pub fn current(&self) -> &[f32] {
        &self.current
    }

    /// Highest persons/m² each cell has reached, row-major.
    pub fn peak(&self) -> &[f32] {
        &self.peak
    }

    /// Highest density anywhere, this tick.
    pub fn max_current(&self) -> f32 {
        self.current.iter().copied().fold(0.0, f32::max)
    }

    /// Highest density reached anywhere, ever.
    pub fn max_peak(&self) -> f32 {
        self.peak.iter().copied().fold(0.0, f32::max)
    }

    /// Walkable area, in m², currently above `threshold` persons/m².
    ///
    /// This is the figure that answers "how much of my venue is dangerous",
    /// which is more actionable than a single peak value.
    pub fn area_above(&self, threshold: f32) -> f64 {
        let a = self.cell * self.cell;
        self.current.iter().filter(|d| **d >= threshold).count() as f64 * a
    }

    /// Area above `threshold` at any point during the run.
    pub fn peak_area_above(&self, threshold: f32) -> f64 {
        let a = self.cell * self.cell;
        self.peak.iter().filter(|d| **d >= threshold).count() as f64 * a
    }

    pub fn reset(&mut self) {
        self.current.fill(0.0);
        self.peak.fill(0.0);
    }

    /// Recompute from current agent positions.
    pub fn accumulate(&mut self, w: &World) {
        self.current.fill(0.0);
        let k = 2 * self.kernel_radius + 1;
        // Convert accumulated persons-per-cell into persons per m².
        let per_area = 1.0 / (self.cell * self.cell) as f32;

        for i in 0..w.len() {
            if !w.active[i] {
                continue;
            }
            let cx = ((w.pos_x[i] as f64 - self.origin_x) / self.cell).floor() as i32;
            let cy = ((w.pos_y[i] as f64 - self.origin_y) / self.cell).floor() as i32;

            for dy in -self.kernel_radius..=self.kernel_radius {
                let y = cy + dy;
                if y < 0 || y >= self.rows as i32 {
                    continue;
                }
                let krow = (dy + self.kernel_radius) * k;
                for dx in -self.kernel_radius..=self.kernel_radius {
                    let x = cx + dx;
                    if x < 0 || x >= self.cols as i32 {
                        continue;
                    }
                    let kw = self.kernel[(krow + dx + self.kernel_radius) as usize];
                    self.current[y as usize * self.cols + x as usize] += kw * per_area;
                }
            }
        }

        for (p, c) in self.peak.iter_mut().zip(&self.current) {
            if *c > *p {
                *p = *c;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{AgentState, SpawnParams};
    use cf_geom::Vec2;

    fn bounds() -> Aabb {
        Aabb {
            min: Vec2::new(0.0, 0.0),
            max: Vec2::new(20.0, 12.0),
        }
    }

    fn world_with(points: &[(f64, f64)]) -> World {
        let mut w = World::new();
        for (x, y) in points {
            w.spawn(SpawnParams {
                position: Vec2::new(*x, *y),
                radius_m: 0.23,
                desired_speed: 1.34,
                goal: 0,
                population: 0,
                entry: 0,
                state: AgentState::Walking,
            });
        }
        w
    }

    /// The kernel must conserve mass: summing density x cell area over the whole
    /// grid returns the agent count. Without this the absolute numbers are
    /// meaningless, however plausible the picture looks.
    #[test]
    fn total_mass_equals_the_agent_count() {
        let mut g = DensityGrid::new(bounds(), 0.5);
        let w = world_with(&[(5.0, 5.0), (10.0, 6.0), (15.0, 3.0), (2.0, 2.0)]);
        g.accumulate(&w);

        let area = (g.cell_size() * g.cell_size()) as f32;
        let total: f32 = g.current().iter().map(|d| d * area).sum();
        assert!(
            (total - 4.0).abs() < 0.01,
            "grid holds {total} persons, expected 4"
        );
    }

    #[test]
    fn an_empty_venue_has_zero_density() {
        let mut g = DensityGrid::new(bounds(), 0.5);
        g.accumulate(&World::new());
        assert_eq!(g.max_current(), 0.0);
        assert_eq!(g.area_above(1.0), 0.0);
    }

    /// The reason for smoothing. A tight group must read as one dense region,
    /// not four quarter-density cells — under-reporting a crush risk is the
    /// worst error this module can make.
    #[test]
    fn a_tight_group_reads_as_dense() {
        let mut g = DensityGrid::new(bounds(), 0.5);
        // Nine people inside one square metre: roughly 9 persons/m², well past
        // the 6 p/m² crush threshold.
        let mut pts = Vec::new();
        for i in 0..9 {
            pts.push((10.0 + (i % 3) as f64 * 0.33, 6.0 + (i / 3) as f64 * 0.33));
        }
        let w = world_with(&pts);
        g.accumulate(&w);

        assert!(
            g.max_current() > BAND_CRITICAL,
            "9 people in 1 m² read as {:.2} p/m², below the {BAND_CRITICAL} threshold",
            g.max_current()
        );
    }

    /// A group straddling a cell boundary must read the same as one centred in
    /// a cell. Raw binning fails this, which is the whole argument for a kernel.
    #[test]
    fn density_does_not_depend_on_grid_alignment() {
        let mut centred = DensityGrid::new(bounds(), 0.5);
        let mut straddling = DensityGrid::new(bounds(), 0.5);

        let group = |ox: f64, oy: f64| {
            let mut pts = Vec::new();
            for i in 0..9 {
                pts.push((ox + (i % 3) as f64 * 0.3, oy + (i / 3) as f64 * 0.3));
            }
            world_with(&pts)
        };

        // Cell centres sit at origin + (n + 0.5) * 0.5.
        centred.accumulate(&group(10.25, 6.25));
        straddling.accumulate(&group(10.5, 6.5));

        let a = centred.max_current();
        let b = straddling.max_current();
        assert!(
            (a - b).abs() / a.max(b) < 0.15,
            "alignment changed peak density from {a:.2} to {b:.2}"
        );
    }

    #[test]
    fn peak_is_retained_after_the_crowd_disperses() {
        let mut g = DensityGrid::new(bounds(), 0.5);
        let mut pts = Vec::new();
        for i in 0..9 {
            pts.push((10.0 + (i % 3) as f64 * 0.33, 6.0 + (i / 3) as f64 * 0.33));
        }
        g.accumulate(&world_with(&pts));
        let peak = g.max_peak();
        assert!(peak > BAND_CRITICAL);

        // Everyone leaves.
        g.accumulate(&World::new());
        assert_eq!(g.max_current(), 0.0, "current must follow the crowd");
        assert_eq!(
            g.max_peak(),
            peak,
            "peak must survive — the dangerous moment still happened"
        );
    }

    #[test]
    fn area_above_a_threshold_is_measured_in_square_metres() {
        let mut g = DensityGrid::new(bounds(), 0.5);
        let mut pts = Vec::new();
        for i in 0..25 {
            pts.push((10.0 + (i % 5) as f64 * 0.3, 6.0 + (i / 5) as f64 * 0.3));
        }
        g.accumulate(&world_with(&pts));

        let dense = g.area_above(BAND_CRITICAL);
        assert!(
            dense > 0.0,
            "a 25-person cluster must exceed the threshold somewhere"
        );
        // The cluster spans ~1.2 x 1.2 m; with the kernel's spread, its dangerous
        // footprint cannot plausibly exceed a few square metres.
        assert!(
            dense < 12.0,
            "dangerous area {dense:.1} m² is implausibly large"
        );
        assert!(g.area_above(1000.0) == 0.0, "nothing reaches 1000 p/m²");
    }

    #[test]
    fn agents_outside_the_grid_do_not_panic() {
        let mut g = DensityGrid::new(bounds(), 0.5);
        let w = world_with(&[(-500.0, -500.0), (900.0, 900.0), (10.0, 6.0)]);
        g.accumulate(&w);
        // The one inside still registers.
        assert!(g.max_current() > 0.0);
    }

    #[test]
    fn accumulate_is_reproducible() {
        let w = world_with(&[(5.0, 5.0), (5.3, 5.1), (5.6, 4.9)]);
        let mut a = DensityGrid::new(bounds(), 0.5);
        let mut b = DensityGrid::new(bounds(), 0.5);
        a.accumulate(&w);
        b.accumulate(&w);
        for (x, y) in a.current().iter().zip(b.current()) {
            assert_eq!(x.to_bits(), y.to_bits());
        }
    }

    #[test]
    fn reset_clears_both_current_and_peak() {
        let mut g = DensityGrid::new(bounds(), 0.5);
        g.accumulate(&world_with(&[(10.0, 6.0)]));
        assert!(g.max_peak() > 0.0);
        g.reset();
        assert_eq!(g.max_current(), 0.0);
        assert_eq!(g.max_peak(), 0.0);
    }
}
