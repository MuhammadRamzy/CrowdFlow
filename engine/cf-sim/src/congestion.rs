//! Where a crowd loses time, and how much.
//!
//! # Why delay rather than density
//!
//! A density map answers "where were people packed", which is not the same
//! question as "where did the building cost them time". A packed foyer that
//! keeps moving is not a bottleneck; a half-empty corridor where everyone
//! shuffles is. Density is also already reported, and reporting it twice under
//! two names would be dressing one measurement up as two findings.
//!
//! What is accumulated here is **person-seconds lost**: for every agent, every
//! tick, the time it did not make progress it wanted to make. Summed over a
//! run and binned spatially, that ranks the places worth widening — and it is
//! in units a planner can act on, because a doorway costing 400 person-seconds
//! is costing about seven person-minutes and the sentence writes itself.
//!
//! # What it deliberately does not do
//!
//! It does not decide what a bottleneck *is*. There is no threshold here and no
//! classification, because where the line falls between "busy" and "obstructed"
//! is a judgement that belongs to a fire engineer looking at a specific venue,
//! not to a constant in a crowd simulator. This ranks and reports; someone else
//! draws the line.

use crate::world::World;
use cf_geom::Aabb;

/// A spatial accumulator for time lost to congestion.
#[derive(Clone, Debug)]
pub struct CongestionMap {
    origin_x: f64,
    origin_y: f64,
    cell: f64,
    cols: usize,
    rows: usize,
    /// Person-seconds lost in each cell, over the whole run.
    lost: Vec<f64>,
    /// Total person-seconds lost anywhere, so a share can be quoted.
    total_lost: f64,
}

/// One place worth looking at, with what it cost.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hotspot {
    /// Centre of the cell, metres.
    pub x: f64,
    pub y: f64,
    /// Person-seconds lost here.
    pub lost_person_s: f64,
    /// Share of all time lost in the venue, 0..1.
    pub share: f64,
}

impl CongestionMap {
    /// `cell` is the bin size in metres.
    ///
    /// Coarser than the density grid on purpose. A bottleneck is a place a
    /// person could point at — a doorway, a corner, the head of a stair — and
    /// binning at half a metre would split one doorway across four cells and
    /// rank each of them separately.
    pub fn new(bounds: Aabb, cell: f64) -> Self {
        let cell = cell.max(0.1);
        let pad = 1.0;
        let origin_x = bounds.min.x - pad;
        let origin_y = bounds.min.y - pad;
        let cols = (((bounds.max.x + pad) - origin_x) / cell).ceil().max(1.0) as usize;
        let rows = (((bounds.max.y + pad) - origin_y) / cell).ceil().max(1.0) as usize;
        Self {
            origin_x,
            origin_y,
            cell,
            cols,
            rows,
            lost: vec![0.0; cols * rows],
            total_lost: 0.0,
        }
    }

    pub fn cell_size(&self) -> f64 {
        self.cell
    }

    /// Total person-seconds lost across the venue.
    pub fn total_lost_person_s(&self) -> f64 {
        self.total_lost
    }

    /// Record one tick.
    ///
    /// The shortfall is measured against what each agent was *trying* to do
    /// this tick, not against its free walking speed. An agent slowed by the
    /// density law is walking as fast as the model says a crowd at that density
    /// walks — that is the crowd behaving, not the building failing, and
    /// charging it to the building would make every busy venue look obstructed.
    pub fn accumulate(&mut self, w: &World, dt: f64) {
        if dt <= 0.0 {
            return;
        }
        for i in 0..w.len() {
            if !w.active[i] || !w.state[i].is_mobile() {
                continue;
            }
            let wanted = (w.des_x[i] as f64).hypot(w.des_y[i] as f64);
            if wanted <= 1e-3 {
                continue;
            }
            let achieved = w.speed(i as u32);
            let shortfall = (wanted - achieved).max(0.0);
            if shortfall <= 1e-6 {
                continue;
            }
            // Seconds of progress lost, expressed as time rather than distance:
            // a metre not walked is worth more to a slow walker than a fast one,
            // and it is time an evacuation is measured in.
            let lost = (shortfall / wanted) * dt;

            let Some(idx) = self.index(w.pos_x[i] as f64, w.pos_y[i] as f64) else {
                continue;
            };
            self.lost[idx] += lost;
            self.total_lost += lost;
        }
    }

    /// The worst `n` places, most costly first.
    ///
    /// Cells contributing nothing are omitted rather than returned with a zero,
    /// so an empty run yields an empty list instead of a table of places where
    /// nothing happened.
    pub fn hotspots(&self, n: usize) -> Vec<Hotspot> {
        let mut ranked: Vec<(usize, f64)> = self
            .lost
            .iter()
            .enumerate()
            .filter(|(_, v)| **v > 0.0)
            .map(|(i, v)| (i, *v))
            .collect();

        // Descending by cost, then by index — so two cells that cost the same
        // come back in the same order every run. Sorting by float alone leaves
        // ties to the sort's internal state, which is not reproducible.
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        ranked.truncate(n);

        ranked
            .into_iter()
            .map(|(i, lost)| {
                let col = i % self.cols;
                let row = i / self.cols;
                Hotspot {
                    x: self.origin_x + (col as f64 + 0.5) * self.cell,
                    y: self.origin_y + (row as f64 + 0.5) * self.cell,
                    lost_person_s: lost,
                    share: if self.total_lost > 0.0 {
                        lost / self.total_lost
                    } else {
                        0.0
                    },
                }
            })
            .collect()
    }

    fn index(&self, x: f64, y: f64) -> Option<usize> {
        let col = ((x - self.origin_x) / self.cell).floor();
        let row = ((y - self.origin_y) / self.cell).floor();
        if col < 0.0 || row < 0.0 {
            return None;
        }
        let (col, row) = (col as usize, row as usize);
        if col >= self.cols || row >= self.rows {
            return None;
        }
        Some(row * self.cols + col)
    }
}
