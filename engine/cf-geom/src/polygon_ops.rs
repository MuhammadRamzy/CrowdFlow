//! Polygon validity and containment.
//!
//! The navmesh compiler needs to know whether a polygon is usable *before* it
//! triangulates: a self-intersecting boundary or a repeated vertex will produce
//! a malformed mesh rather than an error, and the malformation surfaces later
//! as agents leaking through geometry.
//!
//! [`validate`] is the gate. It returns every defect it finds rather than the
//! first, so the editor's validation panel can list them all at once.

use crate::predicates::{orient, Orientation};
use crate::primitives::{Polygon, Vec2};
use crate::segment::{Intersection, Segment};

/// Ring winding direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Winding {
    CounterClockwise,
    Clockwise,
    /// Zero area — fewer than three distinct points, or all collinear.
    Degenerate,
}

/// A structural defect in a polygon.
#[derive(Clone, Debug, PartialEq)]
pub enum Defect {
    /// Fewer than three vertices.
    TooFewVertices { count: usize },
    /// A coordinate is NaN or infinite.
    NonFinite { index: usize },
    /// Two consecutive vertices are identical.
    RepeatedVertex { index: usize },
    /// Two non-adjacent edges cross. The polygon is not simple.
    SelfIntersection { edge_a: usize, edge_b: usize },
    /// The ring encloses no area.
    ZeroArea,
}

impl std::fmt::Display for Defect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Defect::TooFewVertices { count } => {
                write!(f, "polygon has {count} vertices, needs at least 3")
            }
            Defect::NonFinite { index } => write!(f, "vertex {index} is not finite"),
            Defect::RepeatedVertex { index } => {
                write!(f, "vertex {index} repeats the previous vertex")
            }
            Defect::SelfIntersection { edge_a, edge_b } => {
                write!(f, "edges {edge_a} and {edge_b} intersect")
            }
            Defect::ZeroArea => write!(f, "polygon encloses zero area"),
        }
    }
}

/// Winding direction of a ring, decided by its exact signed area.
pub fn winding(poly: &Polygon) -> Winding {
    let n = poly.len();
    if n < 3 {
        return Winding::Degenerate;
    }
    let a = poly.signed_area();
    if a > 0.0 {
        Winding::CounterClockwise
    } else if a < 0.0 {
        Winding::Clockwise
    } else {
        Winding::Degenerate
    }
}

/// Reverse a ring's winding in place.
pub fn reverse(poly: &mut Polygon) {
    poly.0.reverse();
}

/// Return the ring wound counter-clockwise.
///
/// The navmesh builder requires CCW outer boundaries; imported geometry comes in
/// either direction depending on how the original CAD tool wrote it.
pub fn to_ccw(poly: &Polygon) -> Polygon {
    let mut p = poly.clone();
    if winding(&p) == Winding::Clockwise {
        reverse(&mut p);
    }
    p
}

/// Is the polygon convex? Degenerate rings are not convex.
pub fn is_convex(poly: &Polygon) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let pts = poly.points();
    let mut seen: Option<Orientation> = None;
    for i in 0..n {
        let o = orient(pts[i], pts[(i + 1) % n], pts[(i + 2) % n]);
        if o.is_collinear() {
            continue;
        }
        match seen {
            None => seen = Some(o),
            Some(prev) if prev != o => return false,
            _ => {}
        }
    }
    seen.is_some()
}

/// Edge `i` of the ring, from vertex `i` to vertex `i+1`.
pub fn edge(poly: &Polygon, i: usize) -> Segment {
    let n = poly.len();
    Segment::new(poly.points()[i], poly.points()[(i + 1) % n])
}

/// Every structural defect in the polygon. Empty means it is a valid simple ring.
///
/// Self-intersection is checked by brute force over edge pairs, which is O(n²).
/// Venue rings are small (tens of vertices); the whole-floor arrangement that
/// genuinely needs a sweep line lives in the import pipeline, not here.
pub fn validate(poly: &Polygon) -> Vec<Defect> {
    let mut defects = Vec::new();
    let n = poly.len();
    let pts = poly.points();

    if n < 3 {
        defects.push(Defect::TooFewVertices { count: n });
        return defects;
    }

    for (i, p) in pts.iter().enumerate() {
        if !p.is_finite() {
            defects.push(Defect::NonFinite { index: i });
        }
    }
    if !defects.is_empty() {
        // Every downstream check would be meaningless on non-finite input.
        return defects;
    }

    for i in 0..n {
        if pts[i] == pts[(i + 1) % n] {
            defects.push(Defect::RepeatedVertex { index: (i + 1) % n });
        }
    }

    for i in 0..n {
        for j in (i + 1)..n {
            // Adjacent edges legitimately share an endpoint; the wrap-around
            // pair (0, n-1) is adjacent too.
            let adjacent = j == i + 1 || (i == 0 && j == n - 1);
            let ei = edge(poly, i);
            let ej = edge(poly, j);

            match ei.intersect(&ej) {
                Intersection::None => {}
                Intersection::Point(p) => {
                    if !adjacent {
                        defects.push(Defect::SelfIntersection {
                            edge_a: i,
                            edge_b: j,
                        });
                    } else {
                        // Adjacent edges may touch only at their shared vertex.
                        let shared = if j == i + 1 { ei.b } else { ei.a };
                        if p != shared {
                            defects.push(Defect::SelfIntersection {
                                edge_a: i,
                                edge_b: j,
                            });
                        }
                    }
                }
                Intersection::Overlap(_) => {
                    // Collinear overlap is always a defect, adjacent or not:
                    // adjacent edges overlapping means the ring doubles back.
                    defects.push(Defect::SelfIntersection {
                        edge_a: i,
                        edge_b: j,
                    });
                }
            }
        }
    }

    if poly.signed_area() == 0.0 {
        defects.push(Defect::ZeroArea);
    }

    defects
}

pub fn is_valid(poly: &Polygon) -> bool {
    validate(poly).is_empty()
}

/// Where a point sits relative to a ring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointLocation {
    Inside,
    Outside,
    /// Exactly on an edge or vertex.
    Boundary,
}

/// Locate a point against a ring, using exact predicates for the boundary case.
///
/// The naive even-odd ray cast is ambiguous on the boundary and can also flip
/// when the ray passes exactly through a vertex. Testing boundary membership
/// first with exact arithmetic removes both problems.
pub fn locate_point(poly: &Polygon, p: Vec2) -> PointLocation {
    let n = poly.len();
    if n < 3 {
        return PointLocation::Outside;
    }

    for i in 0..n {
        if edge(poly, i).contains(p) {
            return PointLocation::Boundary;
        }
    }

    // Winding-number ray cast. Counting crossings by the sign of the exact
    // orientation avoids the vertex-on-ray degeneracy that breaks the naive
    // "is y between" formulation.
    let pts = poly.points();
    let mut winding_number = 0i32;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        if a.y <= p.y {
            if b.y > p.y && orient(a, b, p) == Orientation::CounterClockwise {
                winding_number += 1;
            }
        } else if b.y <= p.y && orient(a, b, p) == Orientation::Clockwise {
            winding_number -= 1;
        }
    }

    if winding_number != 0 {
        PointLocation::Inside
    } else {
        PointLocation::Outside
    }
}

pub fn contains_point(poly: &Polygon, p: Vec2) -> bool {
    locate_point(poly, p) == PointLocation::Inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poly(pts: &[(f64, f64)]) -> Polygon {
        Polygon(pts.iter().map(|(x, y)| Vec2::new(*x, *y)).collect())
    }

    fn square() -> Polygon {
        poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)])
    }

    #[test]
    fn winding_and_reversal() {
        let ccw = square();
        assert_eq!(winding(&ccw), Winding::CounterClockwise);

        let mut cw = ccw.clone();
        reverse(&mut cw);
        assert_eq!(winding(&cw), Winding::Clockwise);

        assert_eq!(winding(&to_ccw(&cw)), Winding::CounterClockwise);
        assert_eq!(winding(&to_ccw(&ccw)), Winding::CounterClockwise);
    }

    #[test]
    fn collinear_ring_is_degenerate() {
        let line = poly(&[(0.0, 0.0), (5.0, 0.0), (10.0, 0.0)]);
        assert_eq!(winding(&line), Winding::Degenerate);
        assert!(validate(&line).contains(&Defect::ZeroArea));
    }

    #[test]
    fn convexity() {
        assert!(is_convex(&square()));
        // An L-shape is not convex.
        let l = poly(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 4.0),
            (4.0, 4.0),
            (4.0, 10.0),
            (0.0, 10.0),
        ]);
        assert!(!is_convex(&l));
        // Convexity does not depend on winding.
        let mut cw = square();
        reverse(&mut cw);
        assert!(is_convex(&cw));
    }

    #[test]
    fn a_square_is_valid() {
        assert!(is_valid(&square()), "{:?}", validate(&square()));
    }

    #[test]
    fn an_l_shape_is_valid() {
        let l = poly(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 4.0),
            (4.0, 4.0),
            (4.0, 10.0),
            (0.0, 10.0),
        ]);
        assert!(is_valid(&l), "{:?}", validate(&l));
    }

    #[test]
    fn a_bowtie_is_self_intersecting() {
        let bowtie = poly(&[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)]);
        let defects = validate(&bowtie);
        assert!(
            defects
                .iter()
                .any(|d| matches!(d, Defect::SelfIntersection { .. })),
            "{defects:?}"
        );
    }

    #[test]
    fn repeated_vertices_are_flagged() {
        let p = poly(&[(0.0, 0.0), (10.0, 0.0), (10.0, 0.0), (10.0, 10.0)]);
        let defects = validate(&p);
        assert!(
            defects
                .iter()
                .any(|d| matches!(d, Defect::RepeatedVertex { .. })),
            "{defects:?}"
        );
    }

    #[test]
    fn too_few_vertices() {
        assert_eq!(
            validate(&poly(&[(0.0, 0.0), (1.0, 1.0)])),
            vec![Defect::TooFewVertices { count: 2 }]
        );
        assert_eq!(
            validate(&Polygon(vec![])),
            vec![Defect::TooFewVertices { count: 0 }]
        );
    }

    #[test]
    fn non_finite_short_circuits() {
        let p = Polygon(vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(f64::NAN, 0.0),
            Vec2::new(1.0, 1.0),
        ]);
        let defects = validate(&p);
        assert_eq!(defects, vec![Defect::NonFinite { index: 1 }]);
    }

    #[test]
    fn point_location_in_a_square() {
        let s = square();
        assert_eq!(locate_point(&s, Vec2::new(5.0, 5.0)), PointLocation::Inside);
        assert_eq!(
            locate_point(&s, Vec2::new(15.0, 5.0)),
            PointLocation::Outside
        );
        assert_eq!(
            locate_point(&s, Vec2::new(-1.0, 5.0)),
            PointLocation::Outside
        );

        // Edges and vertices are boundary, not inside or outside.
        assert_eq!(
            locate_point(&s, Vec2::new(0.0, 5.0)),
            PointLocation::Boundary
        );
        assert_eq!(
            locate_point(&s, Vec2::new(5.0, 0.0)),
            PointLocation::Boundary
        );
        assert_eq!(
            locate_point(&s, Vec2::new(0.0, 0.0)),
            PointLocation::Boundary
        );
        assert_eq!(
            locate_point(&s, Vec2::new(10.0, 10.0)),
            PointLocation::Boundary
        );
    }

    /// The case that breaks naive ray casting: a horizontal ray leaving the test
    /// point passes exactly through a vertex of the ring.
    #[test]
    fn ray_through_a_vertex_is_handled() {
        // Diamond with vertices at the axes.
        let d = poly(&[(0.0, 5.0), (5.0, 0.0), (10.0, 5.0), (5.0, 10.0)]);
        // y = 5 passes exactly through the left and right vertices.
        assert_eq!(locate_point(&d, Vec2::new(5.0, 5.0)), PointLocation::Inside);
        assert_eq!(
            locate_point(&d, Vec2::new(-1.0, 5.0)),
            PointLocation::Outside
        );
        assert_eq!(
            locate_point(&d, Vec2::new(11.0, 5.0)),
            PointLocation::Outside
        );
        assert_eq!(
            locate_point(&d, Vec2::new(0.0, 5.0)),
            PointLocation::Boundary
        );
    }

    #[test]
    fn point_location_is_winding_independent() {
        let ccw = square();
        let mut cw = ccw.clone();
        reverse(&mut cw);
        for p in [
            Vec2::new(5.0, 5.0),
            Vec2::new(15.0, 5.0),
            Vec2::new(0.0, 5.0),
        ] {
            assert_eq!(
                locate_point(&ccw, p),
                locate_point(&cw, p),
                "disagreement at {p:?}"
            );
        }
    }

    #[test]
    fn point_location_in_a_concave_notch() {
        // U-shape: the notch between the arms is outside.
        let u = poly(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (7.0, 10.0),
            (7.0, 3.0),
            (3.0, 3.0),
            (3.0, 10.0),
            (0.0, 10.0),
        ]);
        assert!(is_valid(&u), "{:?}", validate(&u));
        assert_eq!(locate_point(&u, Vec2::new(5.0, 1.0)), PointLocation::Inside);
        assert_eq!(
            locate_point(&u, Vec2::new(5.0, 7.0)),
            PointLocation::Outside
        );
        assert_eq!(locate_point(&u, Vec2::new(1.0, 7.0)), PointLocation::Inside);
        assert_eq!(locate_point(&u, Vec2::new(8.5, 7.0)), PointLocation::Inside);
    }
}
