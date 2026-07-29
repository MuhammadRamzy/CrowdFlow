//! Offsetting polylines and rings outward by a fixed distance.
//!
//! # What this is for
//!
//! A [`Wall`](../../cf_schema/venue/struct.Wall.html) is stored as a *centreline*
//! polyline plus a `thicknessM`. The navmesh needs solid obstacle polygons. This
//! module turns the former into the latter: offset the centreline by half the
//! thickness to each side and join the two sides into a closed ring.
//!
//! # The hard part: reflex corners
//!
//! Offsetting a single segment is trivial — translate it along its normal. The
//! difficulty is the joins. At a corner, the two offset lines meet at a point
//! found by intersecting them, but as the corner angle approaches zero that
//! intersection shoots off to infinity: a 175° turn on a 0.2 m wall produces a
//! ~2.3 m spike.
//!
//! Those spikes are not cosmetic. They become obstacle geometry, they intersect
//! neighbouring walls that are nowhere near them, and the resulting triangulation
//! is wrong in a way that is very hard to trace back to its cause.
//!
//! So joins are **mitered up to a limit and beveled beyond it** — the standard
//! approach, and the same rule stroke rendering uses. [`MITER_LIMIT`] is the
//! ratio of miter length to offset distance beyond which we bevel.
//!
//! # What this deliberately is not
//!
//! This is not a general polygon offsetting library. It does not resolve the
//! self-intersections that arise when a turn is sharp enough that the *inner*
//! offset side folds through itself — the wall would have to overlap its own
//! other half.
//!
//! Measured limit: turns up to **~150° are handled**; beyond ~155°
//! [`offset_polyline_to_ring`] returns [`OffsetError::SelfIntersecting`].
//! Real wall centrelines turn 90° almost always and 45° occasionally, so this
//! is far more headroom than the input requires. Reporting the case is the
//! point — geometry that looks plausible but triangulates wrongly is much worse
//! than an error.

use crate::polygon_ops;
use crate::primitives::{Polygon, Polyline, Vec2};
use crate::segment::Segment;

/// Miter length beyond this multiple of the offset distance is beveled instead.
///
/// 4.0 corresponds to joins sharper than about 29°. Matches the SVG/Canvas
/// default, which is well-tested against exactly this failure mode.
pub const MITER_LIMIT: f64 = 4.0;

/// Why an offset could not be produced.
#[derive(Clone, Debug, PartialEq)]
pub enum OffsetError {
    /// Fewer than two distinct points.
    Degenerate,
    /// The offset distance was zero or negative.
    NonPositiveDistance(f64),
    /// A coordinate was NaN or infinite.
    NonFinite,
    /// The resulting ring intersects itself. This happens when the offset
    /// distance exceeds the local feature size — for wall thickness on real
    /// geometry it should not occur, so it is reported rather than repaired.
    SelfIntersecting,
}

impl std::fmt::Display for OffsetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OffsetError::Degenerate => write!(f, "polyline has fewer than 2 distinct points"),
            OffsetError::NonPositiveDistance(d) => write!(f, "offset distance {d} must be > 0"),
            OffsetError::NonFinite => write!(f, "input contains non-finite coordinates"),
            OffsetError::SelfIntersecting => {
                write!(
                    f,
                    "offset distance exceeds local feature size; result self-intersects"
                )
            }
        }
    }
}

/// Drop consecutive duplicate points. Offsetting needs a well-defined direction
/// for every segment, and a repeated point has none.
fn dedup(points: &[Vec2]) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = Vec::with_capacity(points.len());
    for p in points {
        if out.last().map(|q: &Vec2| q.distance(*p) > 1e-12) != Some(false) {
            out.push(*p);
        }
    }
    out
}

/// Offset one side of an open polyline by `d`, to the **left** of travel.
///
/// Returns the offset vertices in order. Joins are mitered up to
/// [`MITER_LIMIT`] and beveled beyond it, so a bevel contributes two points
/// where a miter contributes one.
fn offset_side(points: &[Vec2], d: f64) -> Vec<Vec2> {
    let n = points.len();
    debug_assert!(n >= 2);

    // Unit normal to the left of each segment.
    let normals: Vec<Vec2> = points
        .windows(2)
        .map(|w| {
            (w[1] - w[0])
                .normalized()
                .map(|t| t.perp())
                .unwrap_or(Vec2::new(0.0, 1.0))
        })
        .collect();

    let mut out = Vec::with_capacity(n + 4);

    // First endpoint: square off along the first segment's normal.
    out.push(points[0] + normals[0] * d);

    // Interior joins.
    for i in 1..n - 1 {
        let n0 = normals[i - 1];
        let n1 = normals[i];

        let a0 = points[i - 1] + n0 * d;
        let a1 = points[i] + n0 * d;
        let b0 = points[i] + n1 * d;
        let b1 = points[i + 1] + n1 * d;

        // Straight-through: the two offset lines are the same line.
        let cross = n0.cross(n1);
        if cross.abs() < 1e-12 {
            if n0.dot(n1) > 0.0 {
                out.push(a1);
            } else {
                // A 180° reversal — the polyline doubles back. Cap it rather
                // than dividing by a near-zero.
                out.push(a1);
                out.push(b0);
            }
            continue;
        }

        // Miter length ratio: 1/sin(θ/2) where θ is the interior angle.
        // Derived from the half-angle between the two normals.
        let cos_half = ((1.0 + n0.dot(n1)) * 0.5).max(0.0).sqrt();
        let miter_ratio = if cos_half > 1e-12 {
            1.0 / cos_half
        } else {
            f64::INFINITY
        };

        if miter_ratio <= MITER_LIMIT {
            // Miter: intersect the two offset lines.
            match line_intersection(a0, a1, b0, b1) {
                Some(p) => out.push(p),
                // Numerically parallel despite the cross-product check.
                None => {
                    out.push(a1);
                    out.push(b0);
                }
            }
        } else {
            // Bevel: cut the corner off with a straight edge.
            out.push(a1);
            out.push(b0);
        }
    }

    // Last endpoint.
    out.push(points[n - 1] + normals[n - 2] * d);
    out
}

/// Intersection of the infinite lines through `a0→a1` and `b0→b1`.
fn line_intersection(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> Option<Vec2> {
    let r = a1 - a0;
    let s = b1 - b0;
    let denom = r.cross(s);
    if denom.abs() < 1e-15 {
        return None;
    }
    let t = (b0 - a0).cross(s) / denom;
    let p = a0 + r * t;
    p.is_finite().then_some(p)
}

/// Turn a wall centreline into a closed obstacle ring of the given width.
///
/// `width` is the full wall thickness; the centreline is offset by `width / 2`
/// to each side. The returned ring is wound counter-clockwise.
///
/// ```
/// use cf_geom::{Vec2, Polyline, offset::offset_polyline_to_ring};
///
/// // A 10 m wall, 0.2 m thick.
/// let wall = Polyline(vec![Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)]);
/// let ring = offset_polyline_to_ring(&wall, 0.2).unwrap();
///
/// assert!((ring.area() - 2.0).abs() < 1e-9); // 10 m x 0.2 m
/// ```
pub fn offset_polyline_to_ring(line: &Polyline, width: f64) -> Result<Polygon, OffsetError> {
    if !width.is_finite() || width <= 0.0 {
        return Err(OffsetError::NonPositiveDistance(width));
    }
    if !line.points().iter().all(|p| p.is_finite()) {
        return Err(OffsetError::NonFinite);
    }

    let pts = dedup(line.points());
    if pts.len() < 2 {
        return Err(OffsetError::Degenerate);
    }

    let half = width * 0.5;

    // Left side forward, then right side backward — which is the left side of
    // the reversed polyline. Together they close the ring.
    let left = offset_side(&pts, half);
    let mut reversed = pts.clone();
    reversed.reverse();
    let right = offset_side(&reversed, half);

    let mut ring: Vec<Vec2> = Vec::with_capacity(left.len() + right.len());
    ring.extend(left);
    ring.extend(right);

    // Collapse any duplicates introduced where the two sides meet at the caps.
    let mut ring = dedup(&ring);
    if ring.len() > 2 && ring[0].distance(ring[ring.len() - 1]) < 1e-12 {
        ring.pop();
    }

    if ring.len() < 3 {
        return Err(OffsetError::Degenerate);
    }

    let poly = polygon_ops::to_ccw(&Polygon(ring));

    // Guard the case this module does not attempt to repair.
    if polygon_ops::validate(&poly)
        .iter()
        .any(|d| matches!(d, polygon_ops::Defect::SelfIntersection { .. }))
    {
        return Err(OffsetError::SelfIntersecting);
    }

    Ok(poly)
}

/// Distance from a point to the nearest point on a polyline.
///
/// Used by the Social Force Model's wall repulsion term, and by import cleanup
/// when deciding whether a stray segment belongs to an existing wall run.
pub fn distance_to_polyline(line: &Polyline, p: Vec2) -> Option<f64> {
    let pts = line.points();
    if pts.is_empty() {
        return None;
    }
    if pts.len() == 1 {
        return Some(pts[0].distance(p));
    }
    pts.windows(2)
        .map(|w| Segment::new(w[0], w[1]).distance_to_point(p))
        .fold(None, |acc: Option<f64>, d| {
            Some(acc.map_or(d, |a| a.min(d)))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl(pts: &[(f64, f64)]) -> Polyline {
        Polyline(pts.iter().map(|(x, y)| Vec2::new(*x, *y)).collect())
    }

    #[test]
    fn straight_wall_becomes_a_rectangle() {
        let wall = pl(&[(0.0, 0.0), (10.0, 0.0)]);
        let ring = offset_polyline_to_ring(&wall, 0.2).unwrap();

        assert_eq!(ring.len(), 4);
        assert!((ring.area() - 2.0).abs() < 1e-9);
        assert_eq!(
            polygon_ops::winding(&ring),
            polygon_ops::Winding::CounterClockwise
        );
        assert!(polygon_ops::is_valid(&ring));

        // The centreline endpoints sit on the ring's boundary.
        assert_eq!(
            polygon_ops::locate_point(&ring, Vec2::new(0.0, 0.0)),
            polygon_ops::PointLocation::Boundary
        );
        // The centreline midpoint is inside.
        assert_eq!(
            polygon_ops::locate_point(&ring, Vec2::new(5.0, 0.0)),
            polygon_ops::PointLocation::Inside
        );
        // A point clear of the wall is outside.
        assert_eq!(
            polygon_ops::locate_point(&ring, Vec2::new(5.0, 1.0)),
            polygon_ops::PointLocation::Outside
        );
    }

    #[test]
    fn wall_thickness_is_respected_on_both_sides() {
        let wall = pl(&[(0.0, 0.0), (10.0, 0.0)]);
        let ring = offset_polyline_to_ring(&wall, 0.4).unwrap();

        // 0.2 m each side of the centreline.
        for y in [0.19, -0.19] {
            assert_eq!(
                polygon_ops::locate_point(&ring, Vec2::new(5.0, y)),
                polygon_ops::PointLocation::Inside,
                "y={y} should be inside"
            );
        }
        for y in [0.21, -0.21] {
            assert_eq!(
                polygon_ops::locate_point(&ring, Vec2::new(5.0, y)),
                polygon_ops::PointLocation::Outside,
                "y={y} should be outside"
            );
        }
    }

    #[test]
    fn right_angle_corner_miters_cleanly() {
        let wall = pl(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
        let ring = offset_polyline_to_ring(&wall, 0.2).unwrap();

        assert!(
            polygon_ops::is_valid(&ring),
            "{:?}",
            polygon_ops::validate(&ring)
        );

        // Two 10 m arms, 0.2 m thick, sharing a 0.2 x 0.2 corner: about
        // 2 * 2.0 - 0.04 = 3.96 m². A mitered join adds a sliver at the outer
        // corner, so allow a little slack.
        let area = ring.area();
        assert!(
            (3.9..=4.05).contains(&area),
            "unexpected area for an L-shaped wall: {area}"
        );

        // Points along both arms' centrelines are inside.
        assert_eq!(
            polygon_ops::locate_point(&ring, Vec2::new(5.0, 0.0)),
            polygon_ops::PointLocation::Inside
        );
        assert_eq!(
            polygon_ops::locate_point(&ring, Vec2::new(10.0, 5.0)),
            polygon_ops::PointLocation::Inside
        );
        // The inside of the corner is not part of the wall.
        assert_eq!(
            polygon_ops::locate_point(&ring, Vec2::new(5.0, 5.0)),
            polygon_ops::PointLocation::Outside
        );
    }

    /// The spike case. Without a miter limit, a sharp join produces a long thin
    /// protrusion that would intersect unrelated geometry metres away.
    ///
    /// 150° is the sharpest turn this module handles (see
    /// [`hairpin_beyond_the_limit_is_reported_not_guessed`]), so it is the
    /// strongest test of the limit.
    #[test]
    fn very_sharp_corner_is_beveled_not_spiked() {
        let rad = 150.0f64.to_radians();
        let wall = pl(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0 + 10.0 * rad.cos(), 10.0 * rad.sin()),
        ]);
        let ring = offset_polyline_to_ring(&wall, 0.2).unwrap();

        assert!(
            polygon_ops::is_valid(&ring),
            "{:?}",
            polygon_ops::validate(&ring)
        );

        // The unmitered join at 150° would sit ~0.77 m past the corner and grow
        // without bound as the angle sharpens. Bounded here means the limit fired.
        let bounds = ring.bounds().unwrap();
        assert!(
            bounds.max.x < 10.5,
            "miter spike escaped: max.x = {}",
            bounds.max.x
        );
    }

    #[test]
    fn miter_limit_engages_progressively() {
        // Sweep the turn angle across everything real architecture contains.
        // Buildings are overwhelmingly 90°, occasionally 45°; 150° is already
        // far past anything a wall centreline does.
        for angle_deg in [10.0f64, 30.0, 45.0, 60.0, 90.0, 120.0, 140.0, 150.0] {
            let rad = angle_deg.to_radians();
            let wall = pl(&[
                (0.0, 0.0),
                (10.0, 0.0),
                (10.0 + 10.0 * rad.cos(), 10.0 * rad.sin()),
            ]);
            let ring = offset_polyline_to_ring(&wall, 0.2)
                .unwrap_or_else(|e| panic!("angle {angle_deg}: {e}"));

            let bounds = ring.bounds().unwrap();
            assert!(
                bounds.width() < 21.0 && bounds.height() < 21.0,
                "angle {angle_deg}: ring blew up to {bounds:?}"
            );
            assert!(
                polygon_ops::is_valid(&ring),
                "angle {angle_deg}: {:?}",
                polygon_ops::validate(&ring)
            );

            // A mitered join is area-neutral: what the inner side loses, the
            // outer side gains. Two 10 m arms at 0.2 m thick is 4.0 m² at every
            // angle, which is a strong check that the join is not leaking area.
            assert!(
                (ring.area() - 4.0).abs() < 1e-9,
                "angle {angle_deg}: area {} != 4.0",
                ring.area()
            );
        }
    }

    /// Past roughly 155° the inner offset side genuinely folds through itself —
    /// the wall would have to overlap its own other half. This module does not
    /// repair that case (see the module docs); it must **report** it rather than
    /// return geometry that looks plausible and triangulates wrongly.
    #[test]
    fn hairpin_beyond_the_limit_is_reported_not_guessed() {
        for angle_deg in [160.0f64, 170.0, 178.0] {
            let rad = angle_deg.to_radians();
            let wall = pl(&[
                (0.0, 0.0),
                (10.0, 0.0),
                (10.0 + 10.0 * rad.cos(), 10.0 * rad.sin()),
            ]);
            assert_eq!(
                offset_polyline_to_ring(&wall, 0.2),
                Err(OffsetError::SelfIntersecting),
                "angle {angle_deg} should be rejected, not approximated"
            );
        }
    }

    #[test]
    fn multi_segment_wall_run() {
        let wall = pl(&[(0.0, 0.0), (5.0, 0.0), (10.0, 0.0), (15.0, 0.0)]);
        let ring = offset_polyline_to_ring(&wall, 0.3).unwrap();
        assert!(polygon_ops::is_valid(&ring));
        // Collinear interior points must not add area.
        assert!(
            (ring.area() - 15.0 * 0.3).abs() < 1e-9,
            "area {}",
            ring.area()
        );
    }

    #[test]
    fn duplicate_points_are_tolerated() {
        let wall = pl(&[(0.0, 0.0), (5.0, 0.0), (5.0, 0.0), (10.0, 0.0)]);
        let ring = offset_polyline_to_ring(&wall, 0.2).unwrap();
        assert!((ring.area() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn bad_input_is_rejected_not_guessed() {
        let wall = pl(&[(0.0, 0.0), (10.0, 0.0)]);
        assert_eq!(
            offset_polyline_to_ring(&wall, 0.0),
            Err(OffsetError::NonPositiveDistance(0.0))
        );
        assert_eq!(
            offset_polyline_to_ring(&wall, -1.0),
            Err(OffsetError::NonPositiveDistance(-1.0))
        );
        assert_eq!(
            offset_polyline_to_ring(&pl(&[(0.0, 0.0)]), 0.2),
            Err(OffsetError::Degenerate)
        );
        assert_eq!(
            offset_polyline_to_ring(&pl(&[(1.0, 1.0), (1.0, 1.0)]), 0.2),
            Err(OffsetError::Degenerate)
        );
        assert_eq!(
            offset_polyline_to_ring(
                &Polyline(vec![Vec2::new(0.0, 0.0), Vec2::new(f64::NAN, 0.0)]),
                0.2
            ),
            Err(OffsetError::NonFinite)
        );
    }

    #[test]
    fn distance_to_polyline_queries() {
        let wall = pl(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);

        assert!((distance_to_polyline(&wall, Vec2::new(5.0, 3.0)).unwrap() - 3.0).abs() < 1e-12);
        assert!((distance_to_polyline(&wall, Vec2::new(13.0, 5.0)).unwrap() - 3.0).abs() < 1e-12);
        assert!(distance_to_polyline(&wall, Vec2::new(10.0, 0.0)).unwrap() < 1e-12);
        // Beyond the far end: distance to the endpoint.
        assert!((distance_to_polyline(&wall, Vec2::new(10.0, 14.0)).unwrap() - 4.0).abs() < 1e-12);

        assert_eq!(distance_to_polyline(&Polyline(vec![]), Vec2::ZERO), None);
        assert_eq!(
            distance_to_polyline(&pl(&[(3.0, 4.0)]), Vec2::ZERO),
            Some(5.0)
        );
    }

    /// The real fixture: the four walls of `hall-two-doors`, each offset to an
    /// obstacle ring. This is exactly what cf-compile will do.
    #[test]
    fn hall_two_doors_walls_offset_cleanly() {
        let walls = [
            pl(&[(0.0, 12.0), (20.0, 12.0)]),
            pl(&[(20.0, 12.0), (20.0, 0.0)]),
            pl(&[(20.0, 0.0), (0.0, 0.0)]),
            pl(&[(0.0, 0.0), (0.0, 12.0)]),
        ];
        for (i, w) in walls.iter().enumerate() {
            let ring =
                offset_polyline_to_ring(w, 0.23).unwrap_or_else(|e| panic!("wall {i} failed: {e}"));
            assert!(
                polygon_ops::is_valid(&ring),
                "wall {i}: {:?}",
                polygon_ops::validate(&ring)
            );
            let expected = w.length() * 0.23;
            assert!(
                (ring.area() - expected).abs() < 1e-9,
                "wall {i}: area {} != {expected}",
                ring.area()
            );
        }
    }
}
