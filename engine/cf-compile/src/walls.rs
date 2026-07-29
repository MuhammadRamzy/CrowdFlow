//! Splitting wall centrelines around their openings.
//!
//! An [`Opening`] is stored parametrically — a position `t` along its parent
//! wall plus a width. Turning that into geometry means cutting the wall's
//! centreline into the solid runs either side of each doorway, and recording
//! the doorway spans themselves.
//!
//! Both halves are needed. The solid runs become wall constraints. The doorway
//! spans become *temporary* constraints during region classification — a door
//! is a gap, and without sealing it the exterior fill leaks straight into the
//! building (see `crate::compile_floor`).

use cf_geom::{Polyline, Vec2};
use cf_schema::ids::OpeningId;
use cf_schema::venue::{Opening, Wall};

/// A doorway span resolved to world coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct DoorGap {
    pub opening: OpeningId,
    /// The two ends of the gap, in order along the wall.
    pub a: Vec2,
    pub b: Vec2,
    /// Actual span length, which may be less than the authored width if the
    /// opening was clamped to fit its wall.
    pub width_m: f64,
}

/// The result of cutting a wall around its openings.
#[derive(Clone, Debug, Default)]
pub struct WallSplit {
    /// Runs of wall that remain solid.
    pub solid: Vec<Vec<Vec2>>,
    /// The doorways cut out of it.
    pub gaps: Vec<DoorGap>,
    /// Openings whose spans overlapped and were merged.
    pub overlaps: Vec<(OpeningId, OpeningId)>,
    /// Openings clamped because they were wider than the wall.
    pub clamped: Vec<OpeningId>,
}

/// Point at a given arc length along a polyline.
fn point_at_arclen(pl: &Polyline, s: f64) -> Option<Vec2> {
    let total = pl.length();
    if total <= f64::EPSILON {
        return pl.points().first().copied();
    }
    pl.point_at(s / total)
}

/// The sub-polyline between two arc lengths, keeping intermediate vertices.
fn sub_polyline(pl: &Polyline, s0: f64, s1: f64) -> Vec<Vec2> {
    let mut out = Vec::new();
    let Some(start) = point_at_arclen(pl, s0) else {
        return out;
    };
    out.push(start);

    let mut acc = 0.0;
    for w in pl.points().windows(2) {
        acc += w[0].distance(w[1]);
        // `acc` is now the arc length at vertex w[1].
        if acc > s0 && acc < s1 {
            out.push(w[1]);
        }
    }

    if let Some(end) = point_at_arclen(pl, s1) {
        if out.last().map(|p| p.distance(end) > 1e-12) != Some(false) {
            out.push(end);
        }
    }
    out
}

/// Cut `wall` around `openings`, which must all reference it.
pub fn split_wall(wall: &Wall, openings: &[&Opening]) -> WallSplit {
    let mut split = WallSplit::default();
    let total = wall.polyline.length();
    if total <= f64::EPSILON {
        return split;
    }

    // Resolve each opening to an arc-length interval, clamped to the wall.
    let mut spans: Vec<(f64, f64, OpeningId)> = Vec::with_capacity(openings.len());
    for op in openings {
        let half = op.width_m * 0.5;
        if op.width_m > total {
            split.clamped.push(op.id.clone());
        }
        let centre = op.t.clamp(0.0, 1.0) * total;
        let s0 = (centre - half).max(0.0);
        let s1 = (centre + half).min(total);
        if s1 - s0 <= f64::EPSILON {
            continue;
        }
        spans.push((s0, s1, op.id.clone()));
    }

    // Sort and merge overlaps. Two doorways occupying the same stretch of wall
    // is a modelling error, but producing crossing geometry from it would be
    // worse than merging and saying so.
    spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut merged: Vec<(f64, f64, OpeningId)> = Vec::with_capacity(spans.len());
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.0 <= last.1 => {
                split.overlaps.push((last.2.clone(), span.2.clone()));
                last.1 = last.1.max(span.1);
            }
            _ => merged.push(span),
        }
    }

    // Solid runs are the complement of the merged spans.
    let mut cursor = 0.0;
    for (s0, s1, id) in &merged {
        if *s0 - cursor > 1e-9 {
            let run = sub_polyline(&wall.polyline, cursor, *s0);
            if run.len() >= 2 {
                split.solid.push(run);
            }
        }
        if let (Some(a), Some(b)) = (
            point_at_arclen(&wall.polyline, *s0),
            point_at_arclen(&wall.polyline, *s1),
        ) {
            split.gaps.push(DoorGap {
                opening: id.clone(),
                a,
                b,
                width_m: s1 - s0,
            });
        }
        cursor = *s1;
    }
    if total - cursor > 1e-9 {
        let run = sub_polyline(&wall.polyline, cursor, total);
        if run.len() >= 2 {
            split.solid.push(run);
        }
    }

    split
}

#[cfg(test)]
mod tests {
    use super::*;
    use cf_schema::ids::WallId;
    use cf_schema::venue::{OpeningKind, Swing, WallKind};

    fn wall(pts: &[(f64, f64)]) -> Wall {
        Wall {
            id: WallId::new("w"),
            layer: None,
            polyline: Polyline(pts.iter().map(|(x, y)| Vec2::new(*x, *y)).collect()),
            thickness_m: 0.2,
            kind: WallKind::Structural,
            permeable: false,
            provenance: None,
        }
    }

    fn opening(id: &str, t: f64, width_m: f64) -> Opening {
        Opening {
            id: OpeningId::new(id),
            wall: WallId::new("w"),
            t,
            width_m,
            kind: OpeningKind::Door,
            swing: Swing::Both,
            is_fire_exit: false,
            capacity_factor: 1.0,
            schedule: Vec::new(),
            provenance: None,
        }
    }

    #[test]
    fn a_wall_with_no_openings_stays_whole() {
        let w = wall(&[(0.0, 0.0), (10.0, 0.0)]);
        let s = split_wall(&w, &[]);
        assert_eq!(s.solid.len(), 1);
        assert_eq!(s.solid[0], vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)]);
        assert!(s.gaps.is_empty());
    }

    #[test]
    fn a_central_door_splits_the_wall_in_two() {
        let w = wall(&[(0.0, 0.0), (10.0, 0.0)]);
        let op = opening("op", 0.5, 2.0);
        let s = split_wall(&w, &[&op]);

        assert_eq!(s.solid.len(), 2);
        assert_eq!(s.solid[0], vec![Vec2::new(0.0, 0.0), Vec2::new(4.0, 0.0)]);
        assert_eq!(s.solid[1], vec![Vec2::new(6.0, 0.0), Vec2::new(10.0, 0.0)]);

        assert_eq!(s.gaps.len(), 1);
        assert_eq!(s.gaps[0].a, Vec2::new(4.0, 0.0));
        assert_eq!(s.gaps[0].b, Vec2::new(6.0, 0.0));
        assert!((s.gaps[0].width_m - 2.0).abs() < 1e-12);
    }

    /// The M1 fixture's south wall: two doorways, and it runs right-to-left so
    /// arc length increases as x decreases.
    #[test]
    fn hall_two_doors_south_wall() {
        let w = wall(&[(20.0, 0.0), (0.0, 0.0)]);
        let east = opening("op_east_door", 0.25, 1.8);
        let west = opening("op_west_door", 0.75, 1.8);
        let s = split_wall(&w, &[&east, &west]);

        assert_eq!(s.solid.len(), 3, "two doors cut a wall into three runs");
        assert_eq!(s.gaps.len(), 2);

        // t = 0.25 is 5 m along, i.e. x = 15; the 1.8 m door spans x 15.9..14.1.
        assert!((s.gaps[0].a.x - 15.9).abs() < 1e-9, "{:?}", s.gaps[0]);
        assert!((s.gaps[0].b.x - 14.1).abs() < 1e-9);
        // t = 0.75 is 15 m along, i.e. x = 5.
        assert!((s.gaps[1].a.x - 5.9).abs() < 1e-9);
        assert!((s.gaps[1].b.x - 4.1).abs() < 1e-9);

        // Solid length plus door width must equal the wall length.
        let solid: f64 = s
            .solid
            .iter()
            .map(|r| r.windows(2).map(|w| w[0].distance(w[1])).sum::<f64>())
            .sum();
        let gaps: f64 = s.gaps.iter().map(|g| g.width_m).sum();
        assert!(
            (solid + gaps - 20.0).abs() < 1e-9,
            "solid {solid} + gaps {gaps}"
        );
    }

    #[test]
    fn a_door_at_the_very_end_leaves_one_run() {
        let w = wall(&[(0.0, 0.0), (10.0, 0.0)]);
        let op = opening("op", 1.0, 2.0);
        let s = split_wall(&w, &[&op]);

        // Clamped to the wall end: gap is 9..10, one solid run 0..9.
        assert_eq!(s.solid.len(), 1);
        assert_eq!(s.solid[0], vec![Vec2::new(0.0, 0.0), Vec2::new(9.0, 0.0)]);
        assert_eq!(s.gaps.len(), 1);
        assert!(
            (s.gaps[0].width_m - 1.0).abs() < 1e-12,
            "half the door is off the end"
        );
    }

    #[test]
    fn overlapping_doors_merge_and_are_reported() {
        let w = wall(&[(0.0, 0.0), (10.0, 0.0)]);
        let a = opening("a", 0.5, 4.0); // 3..7
        let b = opening("b", 0.6, 4.0); // 4..8
        let s = split_wall(&w, &[&a, &b]);

        assert_eq!(s.overlaps.len(), 1);
        assert_eq!(s.gaps.len(), 1, "overlapping spans merge into one gap");
        assert!((s.gaps[0].a.x - 3.0).abs() < 1e-9);
        assert!((s.gaps[0].b.x - 8.0).abs() < 1e-9);
    }

    #[test]
    fn an_opening_wider_than_its_wall_is_clamped_and_reported() {
        let w = wall(&[(0.0, 0.0), (5.0, 0.0)]);
        let op = opening("op", 0.5, 50.0);
        let s = split_wall(&w, &[&op]);

        assert_eq!(s.clamped, vec![OpeningId::new("op")]);
        assert!(s.solid.is_empty(), "the whole wall is doorway");
        assert_eq!(s.gaps.len(), 1);
        assert!((s.gaps[0].width_m - 5.0).abs() < 1e-9);
    }

    /// A doorway spanning a corner must keep the corner vertex in the solid
    /// runs either side of it.
    #[test]
    fn intermediate_vertices_survive_the_split() {
        let w = wall(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
        let op = opening("op", 0.9, 2.0); // 17..19 along a 20 m wall
        let s = split_wall(&w, &[&op]);

        assert_eq!(s.solid.len(), 2);
        // The first run spans the corner, so it must contain three points.
        assert_eq!(
            s.solid[0].len(),
            3,
            "corner vertex was dropped: {:?}",
            s.solid[0]
        );
        assert_eq!(s.solid[0][1], Vec2::new(10.0, 0.0));
    }

    #[test]
    fn a_zero_length_wall_produces_nothing() {
        let w = wall(&[(3.0, 3.0), (3.0, 3.0)]);
        let s = split_wall(&w, &[]);
        assert!(s.solid.is_empty());
        assert!(s.gaps.is_empty());
    }
}
