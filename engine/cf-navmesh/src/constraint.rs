//! Constraint edge insertion — turning a Delaunay triangulation into a
//! *constrained* one.
//!
//! A wall is a constraint: whatever the Delaunay criterion would prefer, the
//! mesh must contain that edge, and no triangle may straddle it. Without this
//! the triangulation happily runs edges straight through walls, and agents
//! path through them.
//!
//! # The algorithm
//!
//! For each constraint `a → b` not already present as an edge:
//!
//! 1. Walk from `a` toward `b`, collecting the triangles the segment crosses.
//!    The walk also builds two vertex chains — those left of `a → b` and those
//!    right of it.
//! 2. Delete the crossed triangles. What remains is a hole bounded by
//!    `a → b` and the two chains: two *pseudo-polygons*, one per side.
//! 3. Re-triangulate each pseudo-polygon, choosing at each step the vertex that
//!    satisfies the Delaunay criterion. The result is Delaunay everywhere the
//!    constraint permits.
//!
//! # Vertices lying on a constraint
//!
//! If some vertex sits exactly on the segment `a → b`, the constraint is split
//! at that vertex and both halves inserted. This is not an exotic case: it is
//! what a T-junction looks like after import, where one wall ends against the
//! middle of another.
//!
//! # Why adjacency is rebuilt rather than patched
//!
//! Incremental neighbour bookkeeping through this operation has a lot of cases
//! and is the usual source of CDT bugs — a single missed link yields a mesh
//! that looks correct until something walks through it. Instead the whole
//! adjacency is rebuilt by hashing edges once at the end. It is O(triangles),
//! runs at compile time rather than per frame, and cannot drift.

use crate::triangulation::{
    edge_key, triangulate_raw, TriIdx, Triangle, Triangulation, TriangulationError, VertIdx,
};
use cf_geom::predicates::{in_circle, orient, Orientation};
use cf_geom::Vec2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConstraintError {
    /// A constraint referenced a vertex index that does not exist.
    UnknownVertex(VertIdx),
    /// A constraint had identical endpoints.
    Degenerate(VertIdx),
    /// The walk failed to reach the target. Indicates a malformed mesh, and is
    /// a bug rather than bad input — reported instead of looping forever.
    WalkFailed { from: VertIdx, to: VertIdx },
}

impl std::fmt::Display for ConstraintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConstraintError::UnknownVertex(v) => {
                write!(f, "constraint references unknown vertex {v}")
            }
            ConstraintError::Degenerate(v) => {
                write!(f, "constraint has identical endpoints ({v}, {v})")
            }
            ConstraintError::WalkFailed { from, to } => {
                write!(f, "constraint walk from {from} to {to} did not terminate")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CdtError {
    Triangulation(TriangulationError),
    Constraint(ConstraintError),
}

impl std::fmt::Display for CdtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CdtError::Triangulation(e) => write!(f, "{e}"),
            CdtError::Constraint(e) => write!(f, "{e}"),
        }
    }
}

impl From<TriangulationError> for CdtError {
    fn from(e: TriangulationError) -> Self {
        CdtError::Triangulation(e)
    }
}

impl From<ConstraintError> for CdtError {
    fn from(e: ConstraintError) -> Self {
        CdtError::Constraint(e)
    }
}

/// Build a constrained Delaunay triangulation.
///
/// `edges` are vertex index pairs into `points` that must appear in the mesh —
/// wall segments, zone boundaries, the floor outline.
///
/// ```
/// use cf_navmesh::{triangulate_constrained};
/// use cf_geom::Vec2;
///
/// // A square with a diagonal wall forced across it.
/// let pts = vec![
///     Vec2::new(0.0, 0.0),
///     Vec2::new(10.0, 0.0),
///     Vec2::new(10.0, 10.0),
///     Vec2::new(0.0, 10.0),
/// ];
/// let mut t = triangulate_constrained(&pts, &[(0, 2)]).unwrap();
/// t.compact(); // drop tombstoned triangles and the super-triangle vertices
///
/// assert_eq!(t.triangles.len(), 2);
/// assert!((t.total_area() - 100.0).abs() < 1e-9);
/// ```
pub fn triangulate_constrained(
    points: &[Vec2],
    edges: &[(VertIdx, VertIdx)],
) -> Result<Triangulation, CdtError> {
    let mut t = triangulate_raw(points)?;

    for &(a, b) in edges {
        if a >= points.len() {
            return Err(ConstraintError::UnknownVertex(a).into());
        }
        if b >= points.len() {
            return Err(ConstraintError::UnknownVertex(b).into());
        }
        if a == b {
            return Err(ConstraintError::Degenerate(a).into());
        }
    }

    for &(a, b) in edges {
        insert_constraint(&mut t, a, b)?;
    }

    t.remove_super_triangle();
    t.rebuild_adjacency();
    Ok(t)
}

/// Force the edge `a—b` into the mesh.
pub fn insert_constraint(
    t: &mut Triangulation,
    a: VertIdx,
    b: VertIdx,
) -> Result<(), ConstraintError> {
    if a == b {
        return Err(ConstraintError::Degenerate(a));
    }
    t.constraints.insert(edge_key(a, b));

    // Already present? Nothing to carve; the flag is set on rebuild.
    if find_edge(t, a, b).is_some() {
        return Ok(());
    }

    let walk = walk_crossed(t, a, b)?;

    match walk {
        Walk::HitVertex(mid) => {
            // A vertex sits exactly on the segment — a T-junction. Split and
            // insert both halves so the mesh honours the whole run.
            t.constraints.remove(&edge_key(a, b));
            insert_constraint(t, a, mid)?;
            insert_constraint(t, mid, b)?;
            Ok(())
        }
        Walk::Crossed {
            triangles,
            left,
            right,
        } => {
            for idx in &triangles {
                t.triangles[*idx].deleted = true;
            }

            // Each side of the constraint is a pseudo-polygon: the chain of
            // vertices plus the constraint edge closing it.
            let mut new_tris: Vec<[VertIdx; 3]> = Vec::new();
            triangulate_pseudo_polygon(t, a, b, &left, &mut new_tris);
            triangulate_pseudo_polygon(t, a, b, &right, &mut new_tris);

            for tri in new_tris {
                let [x, y, z] = tri;
                // Fix winding here rather than threading orientation through
                // the recursion — exact predicates make this free and it
                // removes a whole class of sign bugs.
                let t2 = match orient(t.points[x], t.points[y], t.points[z]) {
                    Orientation::CounterClockwise => Triangle::new(x, y, z),
                    Orientation::Clockwise => Triangle::new(x, z, y),
                    // A degenerate triangle contributes nothing; dropping it is
                    // correct, and keeping it would corrupt the adjacency.
                    Orientation::Collinear => continue,
                };
                t.triangles.push(t2);
            }

            t.rebuild_adjacency();
            Ok(())
        }
    }
}

enum Walk {
    /// A vertex lies exactly on the constraint; split there.
    HitVertex(VertIdx),
    Crossed {
        triangles: Vec<TriIdx>,
        left: Vec<VertIdx>,
        right: Vec<VertIdx>,
    },
}

/// Find a live triangle holding the edge `a—b`, and which of its edges it is.
fn find_edge(t: &Triangulation, a: VertIdx, b: VertIdx) -> Option<(TriIdx, usize)> {
    t.live().find_map(|(idx, tri)| {
        (0..3).find_map(|i| {
            let (x, y) = tri.edge(i);
            ((x == a && y == b) || (x == b && y == a)).then_some((idx, i))
        })
    })
}

/// Walk from `a` toward `b`, collecting crossed triangles and the two vertex
/// chains flanking the segment.
fn walk_crossed(t: &Triangulation, a: VertIdx, b: VertIdx) -> Result<Walk, ConstraintError> {
    let pa = t.points[a];
    let pb = t.points[b];

    // Find the triangle at `a` that the ray toward `b` leaves through.
    let mut current = None;
    for (idx, tri) in t.live() {
        let Some(k) = tri.index_of(a) else { continue };
        let p = tri.v[(k + 1) % 3];
        let q = tri.v[(k + 2) % 3];

        // A vertex of this triangle lying on the segment means a T-junction.
        for v in [p, q] {
            if v != b && between(pa, pb, t.points[v]) {
                return Ok(Walk::HitVertex(v));
            }
        }

        // `b` is inside the wedge at `a` spanned by a→p rotating CCW to a→q.
        let left_of_p = orient(pa, t.points[p], pb);
        let right_of_q = orient(pa, t.points[q], pb);
        if left_of_p != Orientation::Clockwise && right_of_q != Orientation::CounterClockwise {
            current = Some((idx, k, p, q));
            break;
        }
    }

    let Some((start, k, p, q)) = current else {
        return Err(ConstraintError::WalkFailed { from: a, to: b });
    };

    // Orient the first crossed edge so `u` is left of a→b and `w` is right.
    let (mut u, mut w) = if orient(pa, pb, t.points[p]) == Orientation::CounterClockwise {
        (p, q)
    } else {
        (q, p)
    };

    let mut triangles = vec![start];
    let mut left = vec![u];
    let mut right = vec![w];
    let mut from = start;
    let mut edge_i = k; // the edge opposite `a` is the one we cross

    // Bounded so a malformed mesh reports instead of hanging.
    let limit = t.triangles.len() + 4;
    for _ in 0..limit {
        let next = t.triangles[from].n[edge_i];
        if next == crate::triangulation::NO_NEIGHBOUR {
            return Err(ConstraintError::WalkFailed { from: a, to: b });
        }
        triangles.push(next);

        // The vertex of `next` not on the edge we just crossed.
        let tri = &t.triangles[next];
        let Some(r) = tri.v.iter().copied().find(|v| *v != u && *v != w) else {
            return Err(ConstraintError::WalkFailed { from: a, to: b });
        };

        if r == b {
            return Ok(Walk::Crossed {
                triangles,
                left,
                right,
            });
        }

        if between(pa, pb, t.points[r]) {
            return Ok(Walk::HitVertex(r));
        }

        // Continue across whichever of (u,r) or (w,r) the segment exits.
        let (keep_u, keep_w) = if orient(pa, pb, t.points[r]) == Orientation::CounterClockwise {
            left.push(r);
            (r, w)
        } else {
            right.push(r);
            (u, r)
        };
        u = keep_u;
        w = keep_w;

        let Some(i) = edge_index_between(&t.triangles[next], u, w) else {
            return Err(ConstraintError::WalkFailed { from: a, to: b });
        };
        from = next;
        edge_i = i;
    }

    Err(ConstraintError::WalkFailed { from: a, to: b })
}

/// Which edge of `tri` joins vertices `x` and `y`?
fn edge_index_between(tri: &Triangle, x: VertIdx, y: VertIdx) -> Option<usize> {
    (0..3).find(|&i| {
        let (p, q) = tri.edge(i);
        (p == x && q == y) || (p == y && q == x)
    })
}

/// Is `p` strictly between `a` and `b` on the segment `a—b`? Exact.
fn between(a: Vec2, b: Vec2, p: Vec2) -> bool {
    if p == a || p == b {
        return false;
    }
    if !orient(a, b, p).is_collinear() {
        return false;
    }
    let min_x = a.x.min(b.x);
    let max_x = a.x.max(b.x);
    let min_y = a.y.min(b.y);
    let max_y = a.y.max(b.y);
    p.x >= min_x && p.x <= max_x && p.y >= min_y && p.y <= max_y
}

/// Triangulate the pseudo-polygon bounded by the edge `a—b` and `chain`.
///
/// At each step the vertex satisfying the Delaunay criterion against `a—b` is
/// chosen, then the two sub-chains recurse. This keeps the result Delaunay
/// everywhere the constraint allows.
fn triangulate_pseudo_polygon(
    t: &Triangulation,
    a: VertIdx,
    b: VertIdx,
    chain: &[VertIdx],
    out: &mut Vec<[VertIdx; 3]>,
) {
    if chain.is_empty() {
        return;
    }
    if chain.len() == 1 {
        out.push([a, chain[0], b]);
        return;
    }

    // Pick the chain vertex whose circumcircle with a,b contains no other.
    let mut best = 0usize;
    for i in 1..chain.len() {
        let (x, y, z) = ccw_triple(t, a, b, chain[best]);
        if in_circle(t.points[x], t.points[y], t.points[z], t.points[chain[i]]) {
            best = i;
        }
    }
    let c = chain[best];
    out.push([a, c, b]);

    triangulate_pseudo_polygon(t, a, c, &chain[..best], out);
    triangulate_pseudo_polygon(t, c, b, &chain[best + 1..], out);
}

/// Order three vertices counter-clockwise, as `in_circle` requires.
fn ccw_triple(
    t: &Triangulation,
    a: VertIdx,
    b: VertIdx,
    c: VertIdx,
) -> (VertIdx, VertIdx, VertIdx) {
    match orient(t.points[a], t.points[b], t.points[c]) {
        Orientation::Clockwise => (a, c, b),
        _ => (a, b, c),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pts(v: &[(f64, f64)]) -> Vec<Vec2> {
        v.iter().map(|(x, y)| Vec2::new(*x, *y)).collect()
    }

    /// Does the mesh actually contain this edge?
    fn has_edge(t: &Triangulation, a: VertIdx, b: VertIdx) -> bool {
        t.live().any(|(_, tri)| {
            (0..3).any(|i| {
                let (x, y) = tri.edge(i);
                (x == a && y == b) || (x == b && y == a)
            })
        })
    }

    fn constrained_edge_marked(t: &Triangulation, a: VertIdx, b: VertIdx) -> bool {
        t.live().any(|(idx, tri)| {
            (0..3).any(|i| {
                let (x, y) = tri.edge(i);
                ((x == a && y == b) || (x == b && y == a)) && t.is_constrained(idx, i)
            })
        })
    }

    #[test]
    fn constraint_already_present_is_just_marked() {
        let p = pts(&[(0.0, 0.0), (10.0, 0.0), (5.0, 8.0)]);
        let mut t = triangulate_constrained(&p, &[(0, 1)]).unwrap();
        t.compact();

        assert_eq!(t.triangles.len(), 1);
        assert!(has_edge(&t, 0, 1));
        assert!(constrained_edge_marked(&t, 0, 1));
        assert!(
            t.check_invariants().is_empty(),
            "{:?}",
            t.check_invariants()
        );
    }

    /// The square's Delaunay diagonal is arbitrary (the four corners are
    /// cocircular). Forcing the other diagonal proves the constraint wins.
    #[test]
    fn forced_diagonal_appears_in_the_mesh() {
        let p = pts(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);

        for (a, b) in [(0usize, 2usize), (1, 3)] {
            let mut t = triangulate_constrained(&p, &[(a, b)]).unwrap();
            t.compact();

            assert_eq!(t.triangles.len(), 2);
            assert!(has_edge(&t, a, b), "constraint ({a},{b}) missing");
            assert!(constrained_edge_marked(&t, a, b));
            assert!((t.total_area() - 100.0).abs() < 1e-9);
            let errs = t.check_invariants();
            assert!(errs.is_empty(), "{errs:#?}");
        }
    }

    /// A constraint that has to cut across several triangles.
    #[test]
    fn constraint_crossing_many_triangles() {
        let mut p = pts(&[(0.0, 0.0), (40.0, 0.0)]);
        // A row of vertices above and below, so the segment 0-1 is not a
        // Delaunay edge and must carve through the interior.
        for i in 1..8 {
            p.push(Vec2::new(i as f64 * 5.0, 3.0));
            p.push(Vec2::new(i as f64 * 5.0, -3.0));
        }

        let mut t = triangulate_constrained(&p, &[(0, 1)]).unwrap();
        t.compact();

        assert!(has_edge(&t, 0, 1), "long constraint was not inserted");
        assert!(constrained_edge_marked(&t, 0, 1));
        let errs = t.check_invariants();
        assert!(errs.is_empty(), "{errs:#?}");
    }

    /// A vertex sitting exactly on the constraint — what a T-junction looks
    /// like after import. The constraint must split rather than skip it.
    #[test]
    fn vertex_on_the_constraint_splits_it() {
        let p = pts(&[
            (0.0, 0.0),   // 0
            (20.0, 0.0),  // 1
            (10.0, 0.0),  // 2 — exactly on 0—1
            (10.0, 8.0),  // 3
            (10.0, -8.0), // 4
        ]);

        let mut t = triangulate_constrained(&p, &[(0, 1)]).unwrap();
        t.compact();

        // The full span cannot exist as one edge; both halves must.
        assert!(has_edge(&t, 0, 2), "left half missing");
        assert!(has_edge(&t, 2, 1), "right half missing");
        assert!(constrained_edge_marked(&t, 0, 2));
        assert!(constrained_edge_marked(&t, 2, 1));

        let errs = t.check_invariants();
        assert!(errs.is_empty(), "{errs:#?}");
    }

    /// The real shape: a rectangular hall with its four walls constrained.
    /// This is what cf-compile will hand the navmesh for the M1 fixture.
    #[test]
    fn hall_outline_is_fully_constrained() {
        let p = pts(&[(0.0, 0.0), (20.0, 0.0), (20.0, 12.0), (0.0, 12.0)]);
        let walls = [(0, 1), (1, 2), (2, 3), (3, 0)];

        let mut t = triangulate_constrained(&p, &walls).unwrap();
        t.compact();

        assert_eq!(t.triangles.len(), 2);
        assert!((t.total_area() - 240.0).abs() < 1e-9);
        for (a, b) in walls {
            assert!(has_edge(&t, a, b), "wall ({a},{b}) missing");
            assert!(constrained_edge_marked(&t, a, b), "wall ({a},{b}) unmarked");
        }
        let errs = t.check_invariants();
        assert!(errs.is_empty(), "{errs:#?}");
    }

    /// A hall with an interior obstacle, both outlines constrained — the
    /// arrangement cf-compile actually produces for a room with a pillar.
    #[test]
    fn hall_with_an_interior_obstacle() {
        let p = pts(&[
            // Outer
            (0.0, 0.0),
            (20.0, 0.0),
            (20.0, 12.0),
            (0.0, 12.0),
            // Pillar
            (9.0, 5.0),
            (11.0, 5.0),
            (11.0, 7.0),
            (9.0, 7.0),
        ]);
        let edges = [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 0),
            (4, 5),
            (5, 6),
            (6, 7),
            (7, 4),
        ];

        let mut t = triangulate_constrained(&p, &edges).unwrap();
        t.compact();

        for (a, b) in edges {
            assert!(has_edge(&t, a, b), "edge ({a},{b}) missing");
            assert!(constrained_edge_marked(&t, a, b), "edge ({a},{b}) unmarked");
        }
        // The mesh still tiles the full rectangle; the pillar interior is
        // carved out later by flood fill, not by the triangulator.
        assert!(
            (t.total_area() - 240.0).abs() < 1e-9,
            "area {}",
            t.total_area()
        );
        let errs = t.check_invariants();
        assert!(errs.is_empty(), "{errs:#?}");
    }

    #[test]
    fn many_constraints_on_a_grid() {
        let mut p = Vec::new();
        for i in 0..6 {
            for j in 0..6 {
                p.push(Vec2::new(i as f64 * 4.0, j as f64 * 4.0));
            }
        }
        // Constrain the outer ring.
        let idx = |i: usize, j: usize| i * 6 + j;
        let mut edges = Vec::new();
        for i in 0..5 {
            edges.push((idx(i, 0), idx(i + 1, 0)));
            edges.push((idx(i, 5), idx(i + 1, 5)));
            edges.push((idx(0, i), idx(0, i + 1)));
            edges.push((idx(5, i), idx(5, i + 1)));
        }
        // Plus a diagonal cutting the interior. On a regular lattice this
        // passes *exactly through* the intermediate grid points (2,2) and
        // (3,3), so it must arrive as three collinear sub-edges rather than
        // one — the same splitting behaviour a T-junction triggers.
        let ring = edges.clone();
        edges.push((idx(1, 1), idx(4, 4)));

        let mut t = triangulate_constrained(&p, &edges).unwrap();
        t.compact();

        for (a, b) in &ring {
            assert!(has_edge(&t, *a, *b), "ring edge ({a},{b}) missing");
            assert!(constrained_edge_marked(&t, *a, *b));
        }

        // The diagonal exists as its pieces, not as a single span.
        for (a, b) in [
            (idx(1, 1), idx(2, 2)),
            (idx(2, 2), idx(3, 3)),
            (idx(3, 3), idx(4, 4)),
        ] {
            assert!(has_edge(&t, a, b), "diagonal segment ({a},{b}) missing");
            assert!(constrained_edge_marked(&t, a, b));
        }
        assert!(
            !has_edge(&t, idx(1, 1), idx(4, 4)),
            "the full diagonal cannot be one edge — it passes through two vertices"
        );

        assert!(
            (t.total_area() - 400.0).abs() < 1e-9,
            "area {}",
            t.total_area()
        );
        let errs = t.check_invariants();
        assert!(errs.is_empty(), "{errs:#?}");
    }

    #[test]
    fn bad_constraints_are_rejected() {
        let p = pts(&[(0.0, 0.0), (10.0, 0.0), (5.0, 8.0)]);
        assert_eq!(
            triangulate_constrained(&p, &[(0, 9)]).unwrap_err(),
            CdtError::Constraint(ConstraintError::UnknownVertex(9))
        );
        assert_eq!(
            triangulate_constrained(&p, &[(1, 1)]).unwrap_err(),
            CdtError::Constraint(ConstraintError::Degenerate(1))
        );
    }

    #[test]
    fn constraints_survive_random_point_clouds() {
        let mut seed = 0x0803_2026u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) as f64) / ((1u64 << 31) as f64)
        };

        // A ring of hull points plus scattered interior points.
        let mut p = pts(&[(0.0, 0.0), (60.0, 0.0), (60.0, 40.0), (0.0, 40.0)]);
        for _ in 0..60 {
            p.push(Vec2::new(2.0 + next() * 56.0, 2.0 + next() * 36.0));
        }

        let edges = [(0, 1), (1, 2), (2, 3), (3, 0), (0, 2)];
        let mut t = triangulate_constrained(&p, &edges).unwrap();
        t.compact();

        for (a, b) in edges {
            assert!(has_edge(&t, a, b), "constraint ({a},{b}) lost");
            assert!(constrained_edge_marked(&t, a, b));
        }
        assert!(
            (t.total_area() - 2400.0).abs() < 1e-9,
            "area {} != 2400",
            t.total_area()
        );
        let errs = t.check_invariants();
        assert!(errs.is_empty(), "{errs:#?}");
    }
}
