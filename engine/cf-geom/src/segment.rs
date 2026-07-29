//! Line segments: intersection, distance and closest-point queries.
//!
//! Segment intersection is the workhorse of both the navmesh build (wall
//! constraints crossing each other) and the import topology repair (finding
//! where a dangling wall end should be extended to). It is also the classic
//! source of subtle geometry bugs, because the interesting cases are the
//! degenerate ones: touching endpoints, collinear overlap, zero-length inputs.
//!
//! [`Segment::intersect`] classifies all of them explicitly rather than
//! returning an `Option<Vec2>` that silently loses the distinction.

use crate::predicates::{collinear, on_segment_collinear, orient};
use crate::primitives::Vec2;

/// A directed line segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub a: Vec2,
    pub b: Vec2,
}

/// The result of intersecting two segments.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Intersection {
    /// The segments do not meet.
    None,
    /// They meet at exactly one point.
    ///
    /// Note this includes the "touching" cases — a T-junction where one
    /// segment's endpoint lands on the other's interior, and an L-junction
    /// where two endpoints coincide. Topology repair needs to tell these apart
    /// from a true crossing, so use [`Intersection::is_proper`].
    Point(Vec2),
    /// The segments are collinear and overlap in a sub-segment.
    Overlap(Segment),
}

impl Intersection {
    pub fn is_none(self) -> bool {
        matches!(self, Intersection::None)
    }

    pub fn point(self) -> Option<Vec2> {
        match self {
            Intersection::Point(p) => Some(p),
            _ => None,
        }
    }
}

impl Segment {
    pub fn new(a: Vec2, b: Vec2) -> Self {
        Self { a, b }
    }

    pub fn direction(&self) -> Vec2 {
        self.b - self.a
    }

    pub fn length(&self) -> f64 {
        self.a.distance(self.b)
    }

    pub fn is_degenerate(&self) -> bool {
        self.a == self.b
    }

    pub fn midpoint(&self) -> Vec2 {
        self.a.lerp(self.b, 0.5)
    }

    /// Point at parameter `t`, where `t = 0` is `a` and `t = 1` is `b`.
    pub fn at(&self, t: f64) -> Vec2 {
        self.a.lerp(self.b, t)
    }

    /// Does `p` lie on this closed segment? Exact.
    pub fn contains(&self, p: Vec2) -> bool {
        if self.is_degenerate() {
            return p == self.a;
        }
        collinear(self.a, self.b, p) && on_segment_collinear(self.a, self.b, p)
    }

    /// Parameter of the closest point on the segment to `p`, clamped to `[0,1]`.
    pub fn closest_param(&self, p: Vec2) -> f64 {
        let d = self.direction();
        let len_sq = d.dot(d);
        if len_sq <= f64::EPSILON {
            return 0.0;
        }
        ((p - self.a).dot(d) / len_sq).clamp(0.0, 1.0)
    }

    /// The point on this segment closest to `p`.
    ///
    /// This is the query the Social Force Model makes millions of times per
    /// tick against wall geometry, so it stays allocation-free and branch-light.
    pub fn closest_point(&self, p: Vec2) -> Vec2 {
        self.at(self.closest_param(p))
    }

    /// Euclidean distance from `p` to this segment.
    pub fn distance_to_point(&self, p: Vec2) -> f64 {
        self.closest_point(p).distance(p)
    }

    /// Squared distance from `p` to this segment. Prefer this in hot loops —
    /// it avoids a `sqrt` and compares identically.
    pub fn distance_sq_to_point(&self, p: Vec2) -> f64 {
        let c = self.closest_point(p);
        let d = p - c;
        d.dot(d)
    }

    /// Do the two segments cross at a single interior point of both?
    ///
    /// Excludes touching and collinear overlap. This is the test for "these
    /// walls actually cross", as distinct from "these walls meet at a corner".
    pub fn crosses_properly(&self, other: &Segment) -> bool {
        if self.is_degenerate() || other.is_degenerate() {
            return false;
        }
        let d1 = orient(self.a, self.b, other.a);
        let d2 = orient(self.a, self.b, other.b);
        let d3 = orient(other.a, other.b, self.a);
        let d4 = orient(other.a, other.b, self.b);

        !d1.is_collinear()
            && !d2.is_collinear()
            && !d3.is_collinear()
            && !d4.is_collinear()
            && d1 != d2
            && d3 != d4
    }

    /// Full intersection classification.
    ///
    /// Handles every degenerate case explicitly: zero-length segments, shared
    /// endpoints, endpoint-on-interior (T-junctions), and collinear overlap.
    pub fn intersect(&self, other: &Segment) -> Intersection {
        // Degenerate inputs: a zero-length segment is a point.
        match (self.is_degenerate(), other.is_degenerate()) {
            (true, true) => {
                return if self.a == other.a {
                    Intersection::Point(self.a)
                } else {
                    Intersection::None
                };
            }
            (true, false) => {
                return if other.contains(self.a) {
                    Intersection::Point(self.a)
                } else {
                    Intersection::None
                };
            }
            (false, true) => {
                return if self.contains(other.a) {
                    Intersection::Point(other.a)
                } else {
                    Intersection::None
                };
            }
            (false, false) => {}
        }

        let d1 = orient(self.a, self.b, other.a);
        let d2 = orient(self.a, self.b, other.b);
        let d3 = orient(other.a, other.b, self.a);
        let d4 = orient(other.a, other.b, self.b);

        // General case: each segment strictly straddles the other's line.
        if d1 != d2 && d3 != d4 && !d1.is_collinear() && !d2.is_collinear() {
            return Intersection::Point(self.line_intersection_unchecked(other));
        }

        // All four collinear: potential overlap along a shared line.
        if d1.is_collinear() && d2.is_collinear() {
            return self.collinear_overlap(other);
        }

        // Remaining cases are endpoint-touches. Check each explicitly rather
        // than relying on the straddle test, which is inconclusive here.
        if d1.is_collinear() && on_segment_collinear(self.a, self.b, other.a) {
            return Intersection::Point(other.a);
        }
        if d2.is_collinear() && on_segment_collinear(self.a, self.b, other.b) {
            return Intersection::Point(other.b);
        }
        if d3.is_collinear() && on_segment_collinear(other.a, other.b, self.a) {
            return Intersection::Point(self.a);
        }
        if d4.is_collinear() && on_segment_collinear(other.a, other.b, self.b) {
            return Intersection::Point(self.b);
        }

        Intersection::None
    }

    /// Intersection point of the two infinite lines. Only valid when the
    /// segments are known to cross; callers must have established that.
    fn line_intersection_unchecked(&self, other: &Segment) -> Vec2 {
        let r = self.direction();
        let s = other.direction();
        let denom = r.cross(s);
        // Guarded by the caller's straddle test, but stay defensive: returning
        // a midpoint is far better than returning NaN into a triangulation.
        if denom == 0.0 {
            return self.midpoint();
        }
        let t = (other.a - self.a).cross(s) / denom;
        self.at(t)
    }

    /// Overlap of two collinear segments, if any.
    fn collinear_overlap(&self, other: &Segment) -> Intersection {
        // Project everything onto the dominant axis of this segment's direction
        // to get a stable 1-D ordering.
        let d = self.direction();
        let use_x = d.x.abs() >= d.y.abs();
        let key = |p: Vec2| if use_x { p.x } else { p.y };

        let (mut s1, mut e1) = (self.a, self.b);
        if key(s1) > key(e1) {
            std::mem::swap(&mut s1, &mut e1);
        }
        let (mut s2, mut e2) = (other.a, other.b);
        if key(s2) > key(e2) {
            std::mem::swap(&mut s2, &mut e2);
        }

        let lo = if key(s1) >= key(s2) { s1 } else { s2 };
        let hi = if key(e1) <= key(e2) { e1 } else { e2 };

        if key(lo) > key(hi) {
            Intersection::None
        } else if lo == hi {
            Intersection::Point(lo)
        } else {
            Intersection::Overlap(Segment::new(lo, hi))
        }
    }
}

/// Shortest distance between two segments.
pub fn segment_distance(p: &Segment, q: &Segment) -> f64 {
    if !p.intersect(q).is_none() {
        return 0.0;
    }
    // No intersection, so the minimum is attained at one of the four
    // endpoint-to-segment distances.
    p.distance_to_point(q.a)
        .min(p.distance_to_point(q.b))
        .min(q.distance_to_point(p.a))
        .min(q.distance_to_point(p.b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(ax: f64, ay: f64, bx: f64, by: f64) -> Segment {
        Segment::new(Vec2::new(ax, ay), Vec2::new(bx, by))
    }

    #[test]
    fn proper_crossing() {
        let a = seg(0.0, 0.0, 10.0, 10.0);
        let b = seg(0.0, 10.0, 10.0, 0.0);
        assert!(a.crosses_properly(&b));
        match a.intersect(&b) {
            Intersection::Point(p) => {
                assert!((p.x - 5.0).abs() < 1e-12);
                assert!((p.y - 5.0).abs() < 1e-12);
            }
            other => panic!("expected a point, got {other:?}"),
        }
    }

    #[test]
    fn parallel_segments_do_not_meet() {
        let a = seg(0.0, 0.0, 10.0, 0.0);
        let b = seg(0.0, 1.0, 10.0, 1.0);
        assert_eq!(a.intersect(&b), Intersection::None);
        assert!(!a.crosses_properly(&b));
        assert!((segment_distance(&a, &b) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn disjoint_segments_on_the_same_line() {
        let a = seg(0.0, 0.0, 1.0, 0.0);
        let b = seg(2.0, 0.0, 3.0, 0.0);
        assert_eq!(a.intersect(&b), Intersection::None);
        assert!((segment_distance(&a, &b) - 1.0).abs() < 1e-12);
    }

    /// An L-junction: two walls meeting at a shared corner. Extremely common in
    /// imported floor plans, and must not be reported as a proper crossing.
    #[test]
    fn shared_endpoint_is_a_touch_not_a_crossing() {
        let a = seg(0.0, 0.0, 10.0, 0.0);
        let b = seg(10.0, 0.0, 10.0, 10.0);
        assert!(!a.crosses_properly(&b));
        assert_eq!(a.intersect(&b), Intersection::Point(Vec2::new(10.0, 0.0)));
    }

    /// A T-junction: one wall ends on another's interior.
    #[test]
    fn endpoint_on_interior_is_a_touch() {
        let a = seg(0.0, 0.0, 10.0, 0.0);
        let b = seg(5.0, 0.0, 5.0, 8.0);
        assert!(!a.crosses_properly(&b));
        assert_eq!(a.intersect(&b), Intersection::Point(Vec2::new(5.0, 0.0)));
        // Symmetric.
        assert_eq!(b.intersect(&a), Intersection::Point(Vec2::new(5.0, 0.0)));
    }

    #[test]
    fn collinear_overlap_is_reported_as_a_segment() {
        let a = seg(0.0, 0.0, 10.0, 0.0);
        let b = seg(4.0, 0.0, 20.0, 0.0);
        match a.intersect(&b) {
            Intersection::Overlap(o) => {
                assert_eq!(o.a, Vec2::new(4.0, 0.0));
                assert_eq!(o.b, Vec2::new(10.0, 0.0));
            }
            other => panic!("expected overlap, got {other:?}"),
        }
    }

    #[test]
    fn collinear_overlap_survives_reversed_direction() {
        let a = seg(10.0, 0.0, 0.0, 0.0);
        let b = seg(20.0, 0.0, 4.0, 0.0);
        match a.intersect(&b) {
            Intersection::Overlap(o) => {
                let (lo, hi) = if o.a.x <= o.b.x {
                    (o.a, o.b)
                } else {
                    (o.b, o.a)
                };
                assert_eq!(lo, Vec2::new(4.0, 0.0));
                assert_eq!(hi, Vec2::new(10.0, 0.0));
            }
            other => panic!("expected overlap, got {other:?}"),
        }
    }

    #[test]
    fn collinear_touching_at_one_point() {
        let a = seg(0.0, 0.0, 5.0, 0.0);
        let b = seg(5.0, 0.0, 9.0, 0.0);
        assert_eq!(a.intersect(&b), Intersection::Point(Vec2::new(5.0, 0.0)));
    }

    #[test]
    fn one_segment_contained_in_another() {
        let a = seg(0.0, 0.0, 10.0, 0.0);
        let b = seg(3.0, 0.0, 7.0, 0.0);
        match a.intersect(&b) {
            Intersection::Overlap(o) => {
                assert_eq!(o.a, Vec2::new(3.0, 0.0));
                assert_eq!(o.b, Vec2::new(7.0, 0.0));
            }
            other => panic!("expected overlap, got {other:?}"),
        }
    }

    #[test]
    fn degenerate_segments() {
        let point = seg(5.0, 5.0, 5.0, 5.0);
        let line = seg(0.0, 5.0, 10.0, 5.0);
        let elsewhere = seg(0.0, 0.0, 10.0, 0.0);

        assert_eq!(
            point.intersect(&line),
            Intersection::Point(Vec2::new(5.0, 5.0))
        );
        assert_eq!(
            line.intersect(&point),
            Intersection::Point(Vec2::new(5.0, 5.0))
        );
        assert_eq!(point.intersect(&elsewhere), Intersection::None);
        assert_eq!(
            point.intersect(&point),
            Intersection::Point(Vec2::new(5.0, 5.0))
        );
        assert!(!point.crosses_properly(&line));

        let other_point = seg(1.0, 1.0, 1.0, 1.0);
        assert_eq!(point.intersect(&other_point), Intersection::None);
    }

    #[test]
    fn intersection_is_symmetric() {
        let cases = [
            (seg(0.0, 0.0, 10.0, 10.0), seg(0.0, 10.0, 10.0, 0.0)),
            (seg(0.0, 0.0, 10.0, 0.0), seg(5.0, 0.0, 5.0, 5.0)),
            (seg(0.0, 0.0, 1.0, 0.0), seg(5.0, 5.0, 6.0, 6.0)),
            (seg(0.0, 0.0, 10.0, 0.0), seg(10.0, 0.0, 20.0, 0.0)),
        ];
        for (p, q) in cases {
            let a = p.intersect(&q);
            let b = q.intersect(&p);
            match (a, b) {
                (Intersection::None, Intersection::None) => {}
                (Intersection::Point(x), Intersection::Point(y)) => {
                    assert!(x.distance(y) < 1e-12, "{p:?} vs {q:?}: {x:?} != {y:?}")
                }
                (Intersection::Overlap(_), Intersection::Overlap(_)) => {}
                _ => panic!("asymmetric result for {p:?} and {q:?}: {a:?} vs {b:?}"),
            }
        }
    }

    #[test]
    fn closest_point_and_distance() {
        let s = seg(0.0, 0.0, 10.0, 0.0);

        // Perpendicular foot inside the segment.
        assert_eq!(s.closest_point(Vec2::new(4.0, 3.0)), Vec2::new(4.0, 0.0));
        assert!((s.distance_to_point(Vec2::new(4.0, 3.0)) - 3.0).abs() < 1e-12);

        // Beyond the ends: clamps to the endpoints.
        assert_eq!(s.closest_point(Vec2::new(-5.0, 0.0)), Vec2::new(0.0, 0.0));
        assert_eq!(s.closest_point(Vec2::new(50.0, 0.0)), Vec2::new(10.0, 0.0));
        assert!((s.distance_to_point(Vec2::new(13.0, 4.0)) - 5.0).abs() < 1e-12);

        // Squared distance agrees.
        let p = Vec2::new(4.0, 3.0);
        assert!((s.distance_sq_to_point(p) - 9.0).abs() < 1e-12);
    }

    #[test]
    fn closest_point_on_a_degenerate_segment() {
        let s = seg(2.0, 2.0, 2.0, 2.0);
        assert_eq!(s.closest_point(Vec2::new(9.0, 9.0)), Vec2::new(2.0, 2.0));
        assert!((s.distance_to_point(Vec2::new(2.0, 5.0)) - 3.0).abs() < 1e-12);
    }

    #[test]
    fn contains_respects_exactness() {
        let s = seg(0.0, 0.0, 10.0, 10.0);
        assert!(s.contains(Vec2::new(5.0, 5.0)));
        assert!(s.contains(Vec2::new(0.0, 0.0)));
        assert!(s.contains(Vec2::new(10.0, 10.0)));
        assert!(!s.contains(Vec2::new(11.0, 11.0)));
        assert!(!s.contains(Vec2::new(5.0, 5.000000001)));
    }

    #[test]
    fn segment_distance_between_crossing_segments_is_zero() {
        let a = seg(0.0, 0.0, 10.0, 10.0);
        let b = seg(0.0, 10.0, 10.0, 0.0);
        assert_eq!(segment_distance(&a, &b), 0.0);
    }
}
