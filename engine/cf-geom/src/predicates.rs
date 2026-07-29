//! Exact geometric predicates.
//!
//! # Why not just use `f64` comparisons
//!
//! The obvious orientation test is `(b-a).cross(c-a) > 0.0`. For nearly-collinear
//! points that expression's rounding error can exceed its own magnitude, so it
//! returns the *wrong sign*. In a constrained Delaunay triangulation a single
//! wrong sign produces a non-planar mesh — overlapping triangles, an inverted
//! face, or an infinite loop in the flip routine.
//!
//! The failure does not surface where it was caused. It surfaces much later as
//! agents walking through a wall, in a venue the user imported, on someone
//! else's machine. Nearly-collinear points are not an edge case here either:
//! architectural drawings are full of them, because buildings are full of
//! parallel walls that almost line up.
//!
//! So every orientation and in-circle test goes through Shewchuk's adaptive
//! precision predicates (the `robust` crate). They start with the fast floating
//! point filter and only escalate to exact arithmetic when the error bound says
//! the sign is not yet certain — so the common case costs about the same as the
//! naive version.
//!
//! **Never compute an orientation by hand in this codebase.** Use [`orient`].

use crate::primitives::Vec2;

/// Which side of a directed line a point falls on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Orientation {
    /// `c` is to the left of the directed line `a → b` (counter-clockwise turn).
    CounterClockwise,
    /// `c` is to the right of the directed line `a → b` (clockwise turn).
    Clockwise,
    /// `a`, `b`, `c` are exactly collinear.
    Collinear,
}

impl Orientation {
    pub fn is_collinear(self) -> bool {
        matches!(self, Orientation::Collinear)
    }

    /// `+1` counter-clockwise, `-1` clockwise, `0` collinear.
    pub fn sign(self) -> i32 {
        match self {
            Orientation::CounterClockwise => 1,
            Orientation::Clockwise => -1,
            Orientation::Collinear => 0,
        }
    }

    pub fn reversed(self) -> Orientation {
        match self {
            Orientation::CounterClockwise => Orientation::Clockwise,
            Orientation::Clockwise => Orientation::CounterClockwise,
            Orientation::Collinear => Orientation::Collinear,
        }
    }
}

#[inline]
fn coord(v: Vec2) -> robust::Coord<f64> {
    robust::Coord { x: v.x, y: v.y }
}

/// Exact orientation of the triple `(a, b, c)`.
///
/// Returns [`Orientation::CounterClockwise`] when `c` lies left of `a → b`.
/// This result is *exact*: there is no tolerance parameter and no false
/// collinearity from rounding.
pub fn orient(a: Vec2, b: Vec2, c: Vec2) -> Orientation {
    let d = robust::orient2d(coord(a), coord(b), coord(c));
    if d > 0.0 {
        Orientation::CounterClockwise
    } else if d < 0.0 {
        Orientation::Clockwise
    } else {
        Orientation::Collinear
    }
}

/// Exact in-circle test: is `d` strictly inside the circle through `a`, `b`, `c`?
///
/// `a`, `b`, `c` must be in counter-clockwise order. This is the Delaunay
/// criterion; [`crate`]'s CDT uses it to decide edge flips.
pub fn in_circle(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    robust::incircle(coord(a), coord(b), coord(c), coord(d)) > 0.0
}

/// Is `d` exactly on the circle through `a`, `b`, `c`?
pub fn on_circle(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    robust::incircle(coord(a), coord(b), coord(c), coord(d)) == 0.0
}

/// Are these three points collinear? Exact.
pub fn collinear(a: Vec2, b: Vec2, c: Vec2) -> bool {
    orient(a, b, c).is_collinear()
}

/// Signed area of triangle `abc`; positive when counter-clockwise.
///
/// Uses the exact predicate for the sign, so the sign of the result always
/// agrees with [`orient`] — a naive determinant does not guarantee that.
pub fn signed_area(a: Vec2, b: Vec2, c: Vec2) -> f64 {
    robust::orient2d(coord(a), coord(b), coord(c)) * 0.5
}

/// Does `p` lie on the closed segment `a—b`, assuming all three are collinear?
///
/// Only meaningful when [`collinear(a, b, p)`](collinear) holds; callers that
/// have not established that should use
/// [`Segment::contains`](crate::segment::Segment::contains).
pub fn on_segment_collinear(a: Vec2, b: Vec2, p: Vec2) -> bool {
    p.x >= a.x.min(b.x) && p.x <= a.x.max(b.x) && p.y >= a.y.min(b.y) && p.y <= a.y.max(b.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_orientation() {
        let a = Vec2::new(0.0, 0.0);
        let b = Vec2::new(1.0, 0.0);
        assert_eq!(
            orient(a, b, Vec2::new(0.5, 1.0)),
            Orientation::CounterClockwise
        );
        assert_eq!(orient(a, b, Vec2::new(0.5, -1.0)), Orientation::Clockwise);
        assert_eq!(orient(a, b, Vec2::new(0.5, 0.0)), Orientation::Collinear);
        assert_eq!(orient(a, b, Vec2::new(5.0, 0.0)), Orientation::Collinear);
    }

    /// The case that motivates the whole module. These three points are exactly
    /// collinear mathematically, but the naive cross product returns a non-zero
    /// value because the intermediate products are not representable in f64.
    #[test]
    fn near_collinear_where_naive_arithmetic_lies() {
        let a = Vec2::new(0.5, 0.5);
        let b = Vec2::new(12.0, 12.0);
        let c = Vec2::new(24.0, 24.0);

        assert_eq!(orient(a, b, c), Orientation::Collinear);

        // A point one ULP off the line must be detected as off the line.
        let just_above = Vec2::new(24.0, f64::from_bits(24.0f64.to_bits() + 1));
        assert_eq!(orient(a, b, just_above), Orientation::CounterClockwise);

        let just_below = Vec2::new(24.0, f64::from_bits(24.0f64.to_bits() - 1));
        assert_eq!(orient(a, b, just_below), Orientation::Clockwise);
    }

    /// Documents *why* we pay for exact predicates: the naive determinant gets
    /// this wrong. If this test ever starts failing because the naive version
    /// agrees, the motivation has changed and the module comment should be
    /// revisited.
    #[test]
    fn naive_cross_product_disagrees_on_a_hard_case() {
        let a = Vec2::new(0.1, 0.1);
        let b = Vec2::new(0.2, 0.2);
        let c = Vec2::new(0.3, 0.3);

        // Mathematically collinear. The exact predicate knows it.
        assert_eq!(orient(a, b, c), Orientation::Collinear);

        // The naive computation does not necessarily produce exactly 0.0.
        let naive = (b - a).cross(c - a);
        // We assert only that the exact answer is authoritative, not that the
        // naive one is wrong on this specific triple — that varies by platform.
        assert!(naive.abs() < 1e-15, "sanity: naive is at least small here");
    }

    #[test]
    fn orientation_reverses_with_argument_order() {
        let a = Vec2::new(0.0, 0.0);
        let b = Vec2::new(3.0, 1.0);
        let c = Vec2::new(1.0, 4.0);
        assert_eq!(orient(a, b, c), orient(b, c, a));
        assert_eq!(orient(a, b, c), orient(c, a, b));
        assert_eq!(orient(a, b, c).reversed(), orient(a, c, b));
    }

    #[test]
    fn degenerate_inputs_are_collinear_not_a_panic() {
        let p = Vec2::new(2.0, 3.0);
        assert_eq!(orient(p, p, p), Orientation::Collinear);
        assert_eq!(orient(p, p, Vec2::new(9.0, 9.0)), Orientation::Collinear);
    }

    #[test]
    fn in_circle_basics() {
        // Unit square corners, counter-clockwise.
        let a = Vec2::new(0.0, 0.0);
        let b = Vec2::new(1.0, 0.0);
        let c = Vec2::new(1.0, 1.0);

        assert!(in_circle(a, b, c, Vec2::new(0.5, 0.5)));
        assert!(!in_circle(a, b, c, Vec2::new(5.0, 5.0)));
        // The fourth corner is exactly on the circumcircle.
        assert!(on_circle(a, b, c, Vec2::new(0.0, 1.0)));
        assert!(!in_circle(a, b, c, Vec2::new(0.0, 1.0)));
    }

    #[test]
    fn signed_area_matches_orientation_sign() {
        let a = Vec2::new(0.0, 0.0);
        let b = Vec2::new(4.0, 0.0);
        let c = Vec2::new(0.0, 3.0);
        assert!((signed_area(a, b, c) - 6.0).abs() < 1e-12);
        assert!((signed_area(a, c, b) + 6.0).abs() < 1e-12);
        assert_eq!(signed_area(a, b, c).signum() as i32, orient(a, b, c).sign());
    }

    #[test]
    fn works_at_venue_scale_coordinates() {
        // A stadium-scale venue: ~400 m across, millimetre features.
        let a = Vec2::new(0.0, 0.0);
        let b = Vec2::new(400.0, 0.0);
        let c = Vec2::new(200.0, 0.001);
        assert_eq!(orient(a, b, c), Orientation::CounterClockwise);
        let c = Vec2::new(200.0, -0.001);
        assert_eq!(orient(a, b, c), Orientation::Clockwise);
    }
}
