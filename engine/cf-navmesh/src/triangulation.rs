//! Delaunay triangulation via Bowyer–Watson.
//!
//! # Representation
//!
//! Triangles are stored in a flat `Vec` and referred to by index. Each triangle
//! keeps its three vertex indices and its three neighbour triangle indices,
//! where neighbour `i` is the triangle across the edge *opposite* vertex `i`.
//! That opposite-edge convention is the one that makes edge walks read cleanly;
//! it is also the one that is easy to get backwards, so it is stated here and
//! asserted by [`Triangulation::check_invariants`].
//!
//! Deleted triangles are tombstoned rather than removed, so indices stay stable
//! during a build. [`Triangulation::compact`] drops them at the end.
//!
//! # Robustness
//!
//! Every geometric decision goes through the exact predicates in `cf-geom`.
//! Bowyer–Watson is the classic case where inexact arithmetic does not merely
//! produce a slightly-wrong mesh: an incorrect in-circle sign makes the cavity
//! non-star-shaped, and re-triangulating a non-star-shaped cavity yields
//! overlapping triangles or an infinite loop.
//!
//! # Super-triangle
//!
//! The build starts from a triangle large enough to contain every input point,
//! then removes any triangle still touching it. Its vertices are appended to the
//! point list at indices `n..n+3`, so "is a super-triangle vertex" is the test
//! `index >= n`.

use cf_geom::predicates::{in_circle, orient, Orientation};
use cf_geom::{Aabb, Vec2};

/// Index of a triangle within [`Triangulation::triangles`].
pub type TriIdx = usize;
/// Index of a vertex within [`Triangulation::points`].
pub type VertIdx = usize;

/// Sentinel for "no neighbour" — the edge is on the convex hull.
pub const NO_NEIGHBOUR: TriIdx = usize::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Triangle {
    /// Vertex indices, always counter-clockwise.
    pub v: [VertIdx; 3],
    /// `n[i]` is the triangle across the edge opposite vertex `v[i]`,
    /// or [`NO_NEIGHBOUR`].
    pub n: [TriIdx; 3],
    /// `constrained[i]` marks the edge opposite vertex `v[i]` as a constraint
    /// edge — a wall. Agents may not path across it, and the mesh refinement
    /// may not flip it. Derived from [`Triangulation::constraints`] by
    /// [`Triangulation::rebuild_adjacency`].
    pub constrained: [bool; 3],
    /// Tombstone. Compacted away by [`Triangulation::compact`].
    pub deleted: bool,
}

impl Triangle {
    pub(crate) fn new(a: VertIdx, b: VertIdx, c: VertIdx) -> Self {
        Self {
            v: [a, b, c],
            n: [NO_NEIGHBOUR; 3],
            constrained: [false; 3],
            deleted: false,
        }
    }

    /// The edge opposite vertex `i`, as an ordered vertex pair.
    ///
    /// Ordered so the triangle stays on the left, which makes the pairing with
    /// the neighbour's reversed edge unambiguous.
    pub fn edge(&self, i: usize) -> (VertIdx, VertIdx) {
        (self.v[(i + 1) % 3], self.v[(i + 2) % 3])
    }

    /// Position of vertex `v` within this triangle, if present.
    pub fn index_of(&self, v: VertIdx) -> Option<usize> {
        self.v.iter().position(|x| *x == v)
    }

    pub fn contains_vertex(&self, v: VertIdx) -> bool {
        self.v.contains(&v)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriangulationError {
    /// Fewer than three input points.
    TooFewPoints(usize),
    /// All input points are collinear, so there is no triangle to build.
    AllCollinear,
    /// A coordinate was NaN or infinite.
    NonFinite(usize),
    /// Two input points coincide. Callers should deduplicate first — silently
    /// dropping a point would shift every downstream vertex index.
    DuplicatePoint { a: VertIdx, b: VertIdx },
}

impl std::fmt::Display for TriangulationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TriangulationError::TooFewPoints(n) => {
                write!(f, "need at least 3 points, got {n}")
            }
            TriangulationError::AllCollinear => {
                write!(f, "all input points are collinear")
            }
            TriangulationError::NonFinite(i) => write!(f, "point {i} is not finite"),
            TriangulationError::DuplicatePoint { a, b } => {
                write!(
                    f,
                    "points {a} and {b} coincide; deduplicate before triangulating"
                )
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Triangulation {
    /// Input points followed by the three super-triangle vertices during a
    /// build. After [`Triangulation::compact`] only the input points remain.
    pub points: Vec<Vec2>,
    pub triangles: Vec<Triangle>,
    /// Constraint edges as normalised `(min, max)` vertex pairs. The
    /// authoritative record; per-triangle `constrained` flags are derived from
    /// this, so the two can never disagree.
    pub constraints: std::collections::HashSet<(VertIdx, VertIdx)>,
    /// Number of original input points, i.e. where the super-triangle starts.
    pub(crate) num_input: usize,
}

/// Normalise a vertex pair so an edge has one representation regardless of
/// which direction it was discovered from.
pub fn edge_key(a: VertIdx, b: VertIdx) -> (VertIdx, VertIdx) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

impl Triangulation {
    /// Triangles that survive, ignoring tombstones.
    pub fn live(&self) -> impl Iterator<Item = (TriIdx, &Triangle)> {
        self.triangles
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.deleted)
    }

    pub fn live_count(&self) -> usize {
        self.triangles.iter().filter(|t| !t.deleted).count()
    }

    pub fn vertex(&self, v: VertIdx) -> Vec2 {
        self.points[v]
    }

    /// Total area of all live triangles.
    pub fn total_area(&self) -> f64 {
        self.live()
            .map(|(_, t)| {
                let [a, b, c] = t.v;
                cf_geom::predicates::signed_area(self.points[a], self.points[b], self.points[c])
            })
            .sum()
    }
}

/// Build the Delaunay triangulation, leaving the super-triangle in place.
///
/// Constraint insertion runs against this state; call
/// [`Triangulation::remove_super_triangle`] afterwards.
pub(crate) fn triangulate_raw(points: &[Vec2]) -> Result<Triangulation, TriangulationError> {
    if points.len() < 3 {
        return Err(TriangulationError::TooFewPoints(points.len()));
    }
    for (i, p) in points.iter().enumerate() {
        if !p.is_finite() {
            return Err(TriangulationError::NonFinite(i));
        }
    }

    // O(n²), but it runs once per build and n is bounded by the venue's vertex
    // count. Swap for a hash grid if a profile ever says this matters.
    for i in 0..points.len() {
        for j in (i + 1)..points.len() {
            if points[i] == points[j] {
                return Err(TriangulationError::DuplicatePoint { a: i, b: j });
            }
        }
    }

    if points
        .iter()
        .all(|p| orient(points[0], points[1], *p).is_collinear())
    {
        return Err(TriangulationError::AllCollinear);
    }

    let num_input = points.len();
    let mut t = Triangulation {
        points: points.to_vec(),
        triangles: Vec::with_capacity(num_input * 2 + 8),
        constraints: std::collections::HashSet::new(),
        num_input,
    };

    // Super-triangle, sized generously around the input bounds. It only has to
    // contain every point; exactness is not required because all triangles
    // touching it are discarded at the end.
    let bounds = Aabb::of(points.iter().copied()).expect("non-empty");
    let span = bounds.width().max(bounds.height()).max(1.0);
    let cx = (bounds.min.x + bounds.max.x) * 0.5;
    let cy = (bounds.min.y + bounds.max.y) * 0.5;
    let m = span * 20.0;

    t.points.push(Vec2::new(cx - m, cy - m));
    t.points.push(Vec2::new(cx + m, cy - m));
    t.points.push(Vec2::new(cx, cy + m));
    t.triangles
        .push(Triangle::new(num_input, num_input + 1, num_input + 2));

    for v in 0..num_input {
        insert_point(&mut t, v);
    }

    Ok(t)
}

/// Build the Delaunay triangulation of a point set.
///
/// Points must be distinct — see [`TriangulationError::DuplicatePoint`].
pub fn triangulate(points: &[Vec2]) -> Result<Triangulation, TriangulationError> {
    let mut t = triangulate_raw(points)?;
    t.remove_super_triangle();
    Ok(t)
}

impl Triangulation {
    /// Discard every triangle still attached to the super-triangle.
    ///
    /// Must run *after* constraint insertion: a constraint walk that reaches
    /// the convex hull needs the surrounding triangles to still exist.
    pub fn remove_super_triangle(&mut self) {
        let n = self.num_input;
        for tri in &mut self.triangles {
            if tri.v.iter().any(|v| *v >= n) {
                tri.deleted = true;
            }
        }
        drop_dangling_neighbours(self);
    }

    /// Rebuild every neighbour link and constraint flag from scratch by hashing
    /// edges.
    ///
    /// Incremental re-linking during constraint insertion is where CDT
    /// implementations usually go wrong — the bookkeeping has many cases and a
    /// single missed one produces a mesh that looks fine until an agent walks
    /// through a wall. This is O(triangles), runs once per compile rather than
    /// per frame, and cannot get out of step with reality.
    pub fn rebuild_adjacency(&mut self) {
        use std::collections::HashMap;

        let mut edge_owner: HashMap<(VertIdx, VertIdx), Vec<(TriIdx, usize)>> = HashMap::new();
        for (idx, tri) in self.triangles.iter().enumerate() {
            if tri.deleted {
                continue;
            }
            for i in 0..3 {
                let (a, b) = tri.edge(i);
                edge_owner.entry(edge_key(a, b)).or_default().push((idx, i));
            }
        }

        for tri in &mut self.triangles {
            tri.n = [NO_NEIGHBOUR; 3];
            tri.constrained = [false; 3];
        }

        for (key, owners) in &edge_owner {
            // A manifold mesh has at most two triangles per edge.
            if owners.len() == 2 {
                let (t0, i0) = owners[0];
                let (t1, i1) = owners[1];
                self.triangles[t0].n[i0] = t1;
                self.triangles[t1].n[i1] = t0;
            }
            if self.constraints.contains(key) {
                for &(ti, i) in owners {
                    self.triangles[ti].constrained[i] = true;
                }
            }
        }
    }

    /// Is the edge opposite vertex `i` of triangle `t` a constraint?
    pub fn is_constrained(&self, t: TriIdx, i: usize) -> bool {
        self.triangles[t].constrained[i]
    }
}

/// Insert one vertex by the Bowyer–Watson cavity method.
fn insert_point(t: &mut Triangulation, v: VertIdx) {
    let p = t.points[v];

    // 1. Every triangle whose circumcircle contains p forms the cavity.
    let mut bad: Vec<TriIdx> = Vec::new();
    for (idx, tri) in t.triangles.iter().enumerate() {
        if tri.deleted {
            continue;
        }
        let [a, b, c] = tri.v;
        if in_circle(t.points[a], t.points[b], t.points[c], p) {
            bad.push(idx);
        }
    }

    if bad.is_empty() {
        // Can only happen if p is outside the super-triangle, which the sizing
        // rules out. Leaving the mesh untouched is the safe response.
        debug_assert!(false, "point {v} fell outside every circumcircle");
        return;
    }

    // 2. The cavity boundary is every edge not shared by two cavity triangles.
    let in_cavity: std::collections::HashSet<TriIdx> = bad.iter().copied().collect();
    let mut boundary: Vec<(VertIdx, VertIdx, TriIdx)> = Vec::new();
    for &idx in &bad {
        for i in 0..3 {
            let neighbour = t.triangles[idx].n[i];
            let shared = neighbour != NO_NEIGHBOUR && in_cavity.contains(&neighbour);
            if !shared {
                let (a, b) = t.triangles[idx].edge(i);
                boundary.push((a, b, neighbour));
            }
        }
    }

    for &idx in &bad {
        t.triangles[idx].deleted = true;
    }

    // 3. Fan the cavity to p. Each boundary edge becomes one triangle.
    let mut created: Vec<TriIdx> = Vec::with_capacity(boundary.len());
    for &(a, b, outside) in &boundary {
        // The boundary edge is oriented so the cavity is on its left, so
        // (a, b, p) is already counter-clockwise.
        debug_assert!(orient(t.points[a], t.points[b], p) != Orientation::Clockwise);

        let mut tri = Triangle::new(a, b, v);
        // Vertex 2 is p, so the edge opposite vertex 2 is (a, b) — the one
        // facing the triangle outside the cavity.
        tri.n[2] = outside;
        let new_idx = t.triangles.len();
        t.triangles.push(tri);
        created.push(new_idx);

        // Point the outside triangle back at us.
        if outside != NO_NEIGHBOUR {
            for i in 0..3 {
                let (x, y) = t.triangles[outside].edge(i);
                if (x == b && y == a) || (x == a && y == b) {
                    t.triangles[outside].n[i] = new_idx;
                    break;
                }
            }
        }
    }

    // 4. Stitch the new triangles to each other around the fan.
    link_fan(t, &created);
}

/// Connect newly created fan triangles along their shared edges.
///
/// Each new triangle is `(a, b, p)`. Two of them are adjacent when one's `a`
/// equals the other's `b`. Matching on that is O(k²) in the fan size, which is
/// small — cavities are typically a handful of triangles.
fn link_fan(t: &mut Triangulation, created: &[TriIdx]) {
    for i in 0..created.len() {
        for j in (i + 1)..created.len() {
            let ti = created[i];
            let tj = created[j];
            let (ai, bi, _) = (t.triangles[ti].v[0], t.triangles[ti].v[1], ());
            let (aj, bj, _) = (t.triangles[tj].v[0], t.triangles[tj].v[1], ());

            // ti's edge (b_i, p) meets tj's edge (p, a_j) when b_i == a_j.
            // That edge is opposite vertex 0 in both.
            if bi == aj {
                t.triangles[ti].n[0] = tj;
                t.triangles[tj].n[1] = ti;
            } else if bj == ai {
                t.triangles[tj].n[0] = ti;
                t.triangles[ti].n[1] = tj;
            }
        }
    }
}

/// Clear neighbour links that point at deleted triangles.
fn drop_dangling_neighbours(t: &mut Triangulation) {
    let deleted: Vec<bool> = t.triangles.iter().map(|x| x.deleted).collect();
    for tri in &mut t.triangles {
        if tri.deleted {
            continue;
        }
        for k in 0..3 {
            if tri.n[k] != NO_NEIGHBOUR && deleted[tri.n[k]] {
                tri.n[k] = NO_NEIGHBOUR;
            }
        }
    }
}

impl Triangulation {
    /// Remove tombstoned triangles and the super-triangle vertices, renumbering
    /// everything. Call once the build is finished.
    pub fn compact(&mut self) {
        let mut remap = vec![NO_NEIGHBOUR; self.triangles.len()];
        let mut out: Vec<Triangle> = Vec::with_capacity(self.live_count());
        for (old, tri) in self.triangles.iter().enumerate() {
            if !tri.deleted {
                remap[old] = out.len();
                out.push(*tri);
            }
        }
        for tri in &mut out {
            for k in 0..3 {
                tri.n[k] = if tri.n[k] == NO_NEIGHBOUR {
                    NO_NEIGHBOUR
                } else {
                    remap[tri.n[k]]
                };
            }
        }
        self.triangles = out;
        self.points.truncate(self.num_input);
    }

    /// Verify the structural invariants. Returns every violation found.
    ///
    /// Used by tests and debug builds. A triangulation that passes this is
    /// planar and consistently linked; one that does not will produce garbage
    /// paths later, far from the cause.
    pub fn check_invariants(&self) -> Vec<String> {
        let mut errs = Vec::new();

        for (idx, tri) in self.live() {
            let [a, b, c] = tri.v;

            if a == b || b == c || a == c {
                errs.push(format!("triangle {idx} has a repeated vertex: {:?}", tri.v));
                continue;
            }
            for v in tri.v {
                if v >= self.points.len() {
                    errs.push(format!("triangle {idx} references vertex {v} out of range"));
                }
            }
            if errs.len() > 32 {
                return errs;
            }

            // Winding must be counter-clockwise.
            match orient(self.points[a], self.points[b], self.points[c]) {
                Orientation::CounterClockwise => {}
                Orientation::Clockwise => errs.push(format!("triangle {idx} is wound clockwise")),
                Orientation::Collinear => {
                    errs.push(format!("triangle {idx} is degenerate (collinear)"))
                }
            }

            // Neighbour links must be symmetric and edge-consistent.
            for i in 0..3 {
                let nb = tri.n[i];
                if nb == NO_NEIGHBOUR {
                    continue;
                }
                if nb >= self.triangles.len() || self.triangles[nb].deleted {
                    errs.push(format!(
                        "triangle {idx} neighbour {i} points at {nb}, which is not live"
                    ));
                    continue;
                }
                let (x, y) = tri.edge(i);
                let back = &self.triangles[nb];
                let matching = (0..3).find(|&j| {
                    let (p, q) = back.edge(j);
                    (p == y && q == x) || (p == x && q == y)
                });
                match matching {
                    None => errs.push(format!(
                        "triangle {idx} neighbour {i} = {nb} does not share edge ({x},{y})"
                    )),
                    Some(j) if back.n[j] != idx => errs.push(format!(
                        "asymmetric link: {idx}.n[{i}] = {nb} but {nb}.n[{j}] = {}",
                        back.n[j]
                    )),
                    _ => {}
                }
            }
        }

        errs
    }

    /// Does every live triangle satisfy the Delaunay criterion?
    ///
    /// O(triangles × points); for tests only.
    pub fn is_delaunay(&self) -> bool {
        for (_, tri) in self.live() {
            let [a, b, c] = tri.v;
            for v in 0..self.points.len() {
                if tri.contains_vertex(v) {
                    continue;
                }
                if in_circle(
                    self.points[a],
                    self.points[b],
                    self.points[c],
                    self.points[v],
                ) {
                    return false;
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cf_geom::polygon_ops;
    use cf_geom::Polygon;

    fn pts(v: &[(f64, f64)]) -> Vec<Vec2> {
        v.iter().map(|(x, y)| Vec2::new(*x, *y)).collect()
    }

    #[test]
    fn single_triangle() {
        let p = pts(&[(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)]);
        let mut t = triangulate(&p).unwrap();
        t.compact();

        assert_eq!(t.triangles.len(), 1);
        assert!(
            t.check_invariants().is_empty(),
            "{:?}",
            t.check_invariants()
        );
        assert!((t.total_area() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn unit_square_makes_two_triangles() {
        let p = pts(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        let mut t = triangulate(&p).unwrap();
        t.compact();

        assert_eq!(t.triangles.len(), 2);
        assert!(
            t.check_invariants().is_empty(),
            "{:?}",
            t.check_invariants()
        );
        assert!((t.total_area() - 100.0).abs() < 1e-9);
        assert!(t.is_delaunay());
    }

    /// Euler's formula for a triangulated point set: with `n` points of which
    /// `h` are on the convex hull, the triangulation has exactly `2n - h - 2`
    /// triangles. A far stronger check than counting by hand.
    #[test]
    fn triangle_count_matches_eulers_formula() {
        // 4 hull points, 1 interior.
        let p = pts(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 10.0),
            (0.0, 10.0),
            (5.0, 5.0),
        ]);
        let mut t = triangulate(&p).unwrap();
        t.compact();

        let n = 5;
        let h = 4;
        assert_eq!(t.triangles.len(), 2 * n - h - 2);
        assert!(
            t.check_invariants().is_empty(),
            "{:?}",
            t.check_invariants()
        );
        assert!((t.total_area() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn grid_of_points() {
        let mut p = Vec::new();
        for i in 0..6 {
            for j in 0..6 {
                p.push(Vec2::new(i as f64 * 2.0, j as f64 * 2.0));
            }
        }
        let mut t = triangulate(&p).unwrap();
        t.compact();

        assert!(
            t.check_invariants().is_empty(),
            "{:?}",
            t.check_invariants()
        );
        assert!(t.is_delaunay(), "grid triangulation is not Delaunay");

        // The triangulation must exactly tile the 10x10 hull.
        assert!(
            (t.total_area() - 100.0).abs() < 1e-9,
            "area {} != 100",
            t.total_area()
        );
    }

    #[test]
    fn random_points_stay_consistent() {
        // Deterministic LCG — no rand dependency, and a failure is reproducible.
        let mut seed = 0x2026_0803u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 33) as f64) / ((1u64 << 31) as f64)
        };

        let mut p = Vec::new();
        for _ in 0..120 {
            p.push(Vec2::new(next() * 100.0, next() * 100.0));
        }

        let mut t = triangulate(&p).unwrap();
        t.compact();

        let errs = t.check_invariants();
        assert!(errs.is_empty(), "{errs:#?}");
        assert!(t.is_delaunay(), "random triangulation is not Delaunay");

        // Every triangle must be non-degenerate and positively wound.
        for (_, tri) in t.live() {
            let [a, b, c] = tri.v;
            assert_eq!(
                orient(t.points[a], t.points[b], t.points[c]),
                Orientation::CounterClockwise
            );
        }
    }

    /// Points on a common circle are the classic degenerate case: every
    /// in-circle test returns "exactly on", so an inexact predicate would flip
    /// arbitrarily and produce an inconsistent mesh.
    #[test]
    fn cocircular_points_do_not_break_the_mesh() {
        let mut p = Vec::new();
        for i in 0..8 {
            let a = i as f64 * std::f64::consts::TAU / 8.0;
            p.push(Vec2::new(10.0 * a.cos(), 10.0 * a.sin()));
        }
        let mut t = triangulate(&p).unwrap();
        t.compact();

        let errs = t.check_invariants();
        assert!(errs.is_empty(), "{errs:#?}");

        // Regular octagon of circumradius 10: area = 2 * sqrt(2) * r^2.
        let expected = 2.0 * 2.0f64.sqrt() * 100.0;
        assert!(
            (t.total_area() - expected).abs() < 1e-6,
            "area {} != {expected}",
            t.total_area()
        );
    }

    #[test]
    fn nearly_collinear_points() {
        // A very flat sliver plus one point well off the line.
        let p = pts(&[
            (0.0, 0.0),
            (10.0, 1e-9),
            (20.0, 0.0),
            (30.0, 1e-9),
            (15.0, 5.0),
        ]);
        let mut t = triangulate(&p).unwrap();
        t.compact();
        let errs = t.check_invariants();
        assert!(errs.is_empty(), "{errs:#?}");
    }

    #[test]
    fn triangulation_covers_the_convex_hull() {
        let p = pts(&[
            (0.0, 0.0),
            (20.0, 0.0),
            (20.0, 12.0),
            (0.0, 12.0),
            (7.0, 4.0),
            (13.0, 9.0),
        ]);
        let mut t = triangulate(&p).unwrap();
        t.compact();

        assert!((t.total_area() - 240.0).abs() < 1e-9);

        // Every triangle lies inside the hull rectangle.
        let hull = Polygon(pts(&[(0.0, 0.0), (20.0, 0.0), (20.0, 12.0), (0.0, 12.0)]));
        for (idx, tri) in t.live() {
            let [a, b, c] = tri.v;
            let centroid = (t.points[a] + t.points[b] + t.points[c]) * (1.0 / 3.0);
            assert_ne!(
                polygon_ops::locate_point(&hull, centroid),
                polygon_ops::PointLocation::Outside,
                "triangle {idx} centroid {centroid:?} escaped the hull"
            );
        }
    }

    #[test]
    fn bad_input_is_rejected() {
        assert_eq!(
            triangulate(&pts(&[(0.0, 0.0), (1.0, 1.0)])).unwrap_err(),
            TriangulationError::TooFewPoints(2)
        );
        assert_eq!(
            triangulate(&pts(&[(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0)])).unwrap_err(),
            TriangulationError::AllCollinear
        );
        assert_eq!(
            triangulate(&pts(&[(0.0, 0.0), (1.0, 0.0), (0.0, 0.0)])).unwrap_err(),
            TriangulationError::DuplicatePoint { a: 0, b: 2 }
        );
        assert_eq!(
            triangulate(&[
                Vec2::new(0.0, 0.0),
                Vec2::new(f64::NAN, 0.0),
                Vec2::new(1.0, 1.0)
            ])
            .unwrap_err(),
            TriangulationError::NonFinite(1)
        );
    }

    #[test]
    fn hall_two_doors_corners() {
        // The fixture's four corners — the M1 venue.
        let p = pts(&[(0.0, 0.0), (20.0, 0.0), (20.0, 12.0), (0.0, 12.0)]);
        let mut t = triangulate(&p).unwrap();
        t.compact();

        assert_eq!(t.triangles.len(), 2);
        assert!((t.total_area() - 240.0).abs() < 1e-9);
        assert!(t.check_invariants().is_empty());
    }
}
