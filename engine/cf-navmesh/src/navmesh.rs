//! The navigation mesh: walkable triangles, the portals between them, and
//! shortest-path queries across both.
//!
//! # Portals
//!
//! A portal is a shared edge between two walkable triangles. It is where an
//! agent passes from one triangle to the next, and its length is the physical
//! width available at that point. That width is not just a pathfinding detail —
//! it is the figure Green Guide egress capacity is computed from, so it is
//! measured and kept rather than derived later.
//!
//! Constrained edges are never portals: a wall is not a way through.
//!
//! # Pathfinding
//!
//! Two stages, which is the standard decomposition and worth stating because
//! doing only the first is a common mistake:
//!
//! 1. **A\* over the triangle dual graph** finds the *sequence of triangles* to
//!    cross. On its own this yields a centroid-to-centroid path, which zigzags
//!    absurdly — agents would visibly wander.
//! 2. **The funnel algorithm** ("simple stupid funnel") string-pulls that
//!    sequence taut, producing the geometrically shortest path through the
//!    portal sequence. This is what makes agents cut corners the way people do.
//!
//! The result is the true shortest path within the chosen triangle corridor.

use crate::region::{classify, Regions};
use crate::triangulation::{TriIdx, Triangulation, VertIdx, NO_NEIGHBOUR};
use cf_geom::predicates::{orient, Orientation};
use cf_geom::Vec2;

/// A traversable edge between two walkable triangles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Portal {
    pub a: VertIdx,
    pub b: VertIdx,
    /// The two triangles this edge separates.
    pub tris: (TriIdx, TriIdx),
    /// Edge length in metres — the clear width at this crossing.
    pub width: f64,
}

/// A built navigation mesh, ready for queries.
#[derive(Clone, Debug)]
pub struct NavMesh {
    pub tri: Triangulation,
    pub regions: Regions,
    pub portals: Vec<Portal>,
    /// Portal indices touching each triangle, indexed by triangle.
    pub tri_portals: Vec<Vec<usize>>,
    /// Cached centroids, indexed by triangle.
    pub centroids: Vec<Vec2>,
}

impl NavMesh {
    /// Build from a compacted, adjacency-built triangulation.
    pub fn build(tri: Triangulation) -> Self {
        let regions = classify(&tri);
        Self::with_regions(tri, regions)
    }

    pub fn with_regions(tri: Triangulation, regions: Regions) -> Self {
        let n = tri.triangles.len();
        let mut portals = Vec::new();
        let mut tri_portals = vec![Vec::new(); n];
        let mut centroids = vec![Vec2::ZERO; n];

        for (idx, t) in tri.live() {
            let [a, b, c] = t.v;
            centroids[idx] = (tri.points[a] + tri.points[b] + tri.points[c]) * (1.0 / 3.0);
        }

        for (idx, t) in tri.live() {
            if !regions.is_walkable(idx) {
                continue;
            }
            for i in 0..3 {
                let nb = t.n[i];
                // A wall is not a way through, and neither is the edge of the
                // walkable region.
                if nb == NO_NEIGHBOUR || t.constrained[i] || !regions.is_walkable(nb) {
                    continue;
                }
                // Record each shared edge once.
                if nb < idx {
                    continue;
                }
                let (u, w) = t.edge(i);
                let portal = Portal {
                    a: u,
                    b: w,
                    tris: (idx, nb),
                    width: tri.points[u].distance(tri.points[w]),
                };
                let pi = portals.len();
                portals.push(portal);
                tri_portals[idx].push(pi);
                tri_portals[nb].push(pi);
            }
        }

        NavMesh {
            tri,
            regions,
            portals,
            tri_portals,
            centroids,
        }
    }

    pub fn is_walkable(&self, t: TriIdx) -> bool {
        self.regions.is_walkable(t)
    }

    /// The narrowest portal in the mesh — the tightest constriction agents must
    /// pass through. A useful bottleneck signal before any simulation runs.
    pub fn narrowest_portal(&self) -> Option<&Portal> {
        self.portals.iter().min_by(|x, y| {
            x.width
                .partial_cmp(&y.width)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }

    /// Which walkable triangle contains `p`?
    ///
    /// Linear scan. Fine at compile time and for tests; `cf-compile` will add a
    /// uniform grid index for the per-tick lookups the simulation makes.
    pub fn locate(&self, p: Vec2) -> Option<TriIdx> {
        self.tri.live().find_map(|(idx, t)| {
            if !self.regions.is_walkable(idx) {
                return None;
            }
            let [a, b, c] = t.v;
            let (pa, pb, pc) = (self.tri.points[a], self.tri.points[b], self.tri.points[c]);
            // Inside or on the boundary of a CCW triangle.
            let inside = orient(pa, pb, p) != Orientation::Clockwise
                && orient(pb, pc, p) != Orientation::Clockwise
                && orient(pc, pa, p) != Orientation::Clockwise;
            inside.then_some(idx)
        })
    }

    /// The closest point on walkable floor to `p`.
    ///
    /// Returns `p` itself when it is already on floor. Otherwise finds the
    /// nearest point on the boundary of the walkable region.
    ///
    /// This is a recovery path, not a routine query: it scans every walkable
    /// triangle, so it is only affordable because it should almost never run.
    /// An agent outside the mesh means the physics let it escape, and the right
    /// response is to put it back and count the event rather than to lose it.
    pub fn nearest_walkable_point(&self, p: Vec2) -> Option<Vec2> {
        if self.locate(p).is_some() {
            return Some(p);
        }
        let mut best: Option<(f64, Vec2)> = None;
        for (idx, t) in self.tri.live() {
            if !self.regions.is_walkable(idx) {
                continue;
            }
            for i in 0..3 {
                let (a, b) = t.edge(i);
                let seg = cf_geom::Segment::new(self.tri.points[a], self.tri.points[b]);
                let c = seg.closest_point(p);
                let d = c.distance(p);
                if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                    best = Some((d, c));
                }
            }
        }
        best.map(|(_, c)| c)
    }

    /// Shortest walkable path from `from` to `to`, as a polyline including both
    /// endpoints. `None` if either point is outside the walkable region or no
    /// route exists.
    pub fn find_path(&self, from: Vec2, to: Vec2) -> Option<Vec<Vec2>> {
        let start = self.locate(from)?;
        let goal = self.locate(to)?;

        if start == goal {
            return Some(vec![from, to]);
        }

        let corridor = self.astar(start, goal, to)?;
        Some(self.funnel(&corridor, from, to))
    }

    /// A\* over the triangle dual graph. Returns the triangle corridor.
    fn astar(&self, start: TriIdx, goal: TriIdx, goal_pos: Vec2) -> Option<Vec<TriIdx>> {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;

        let n = self.tri.triangles.len();
        let mut came_from = vec![usize::MAX; n];
        let mut g = vec![f64::INFINITY; n];
        // Ordered f64 by bits is fragile; scale to integers for a total order
        // that is also deterministic across targets.
        let key = |v: f64| (v * 1024.0) as u64;

        let mut open = BinaryHeap::new();
        g[start] = 0.0;
        open.push(Reverse((
            key(self.centroids[start].distance(goal_pos)),
            start,
        )));

        while let Some(Reverse((_, cur))) = open.pop() {
            if cur == goal {
                let mut path = vec![goal];
                let mut c = goal;
                while came_from[c] != usize::MAX {
                    c = came_from[c];
                    path.push(c);
                }
                path.reverse();
                return Some(path);
            }

            for &pi in &self.tri_portals[cur] {
                let portal = &self.portals[pi];
                let nb = if portal.tris.0 == cur {
                    portal.tris.1
                } else {
                    portal.tris.0
                };
                // Step cost through the portal midpoint rather than
                // centroid-to-centroid: it tracks the real traversal distance
                // more closely, which matters once portals vary in width.
                let mid = (self.tri.points[portal.a] + self.tri.points[portal.b]) * 0.5;
                let step = self.centroids[cur].distance(mid) + mid.distance(self.centroids[nb]);
                let tentative = g[cur] + step;
                if tentative < g[nb] {
                    g[nb] = tentative;
                    came_from[nb] = cur;
                    let f = tentative + self.centroids[nb].distance(goal_pos);
                    open.push(Reverse((key(f), nb)));
                }
            }
        }
        None
    }

    /// The ordered portal edges crossed by a triangle corridor, each as
    /// `(left, right)` relative to the direction of travel.
    fn corridor_portals(&self, corridor: &[TriIdx]) -> Vec<(Vec2, Vec2)> {
        let mut out = Vec::with_capacity(corridor.len());
        for pair in corridor.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            let t = &self.tri.triangles[from];
            // Exiting `from` through the edge opposite vertex i: by the CCW
            // winding, v[i+1] lies right of travel and v[i+2] lies left.
            for i in 0..3 {
                if t.n[i] == to {
                    let right = self.tri.points[t.v[(i + 1) % 3]];
                    let left = self.tri.points[t.v[(i + 2) % 3]];
                    out.push((left, right));
                    break;
                }
            }
        }
        out
    }

    /// String-pull a triangle corridor into the shortest path through it.
    ///
    /// Mononen's "simple stupid funnel": widen the funnel from the apex until
    /// the left and right bounds cross, at which point the crossing vertex
    /// becomes a corner of the path and the new apex.
    fn funnel(&self, corridor: &[TriIdx], from: Vec2, to: Vec2) -> Vec<Vec2> {
        let mut gates = vec![(from, from)];
        gates.extend(self.corridor_portals(corridor));
        gates.push((to, to));

        let mut path = vec![from];
        let mut apex = from;
        let (mut left, mut right) = (from, from);
        // `apex_i` is only ever read after being written inside a restart
        // branch, so it is deliberately left uninitialised here.
        let mut apex_i;
        let mut left_i = 0usize;
        let mut right_i = 0usize;

        let mut i = 1;
        while i < gates.len() {
            let (gl, gr) = gates[i];

            // Tighten the right bound.
            if area_sign(apex, right, gr) <= 0 {
                if apex == right || area_sign(apex, left, gr) > 0 {
                    right = gr;
                    right_i = i;
                } else {
                    // Right crossed left: left becomes a corner.
                    path.push(left);
                    apex = left;
                    apex_i = left_i;
                    right = apex;
                    left = apex;
                    right_i = apex_i;
                    left_i = apex_i;
                    i = apex_i + 1;
                    continue;
                }
            }

            // Tighten the left bound.
            if area_sign(apex, left, gl) >= 0 {
                if apex == left || area_sign(apex, right, gl) < 0 {
                    left = gl;
                    left_i = i;
                } else {
                    // Left crossed right: right becomes a corner.
                    path.push(right);
                    apex = right;
                    apex_i = right_i;
                    right = apex;
                    left = apex;
                    right_i = apex_i;
                    left_i = apex_i;
                    i = apex_i + 1;
                    continue;
                }
            }

            i += 1;
        }

        if path.last() != Some(&to) {
            path.push(to);
        }
        path
    }
}

/// Sign of Mononen's `triarea2(a, b, c)`, used by the funnel comparisons.
///
/// **This is the negation of the standard CCW orientation.** `triarea2` is
/// defined as `bx*ay - ax*by`, which is `-cross(b-a, c-a)`. Getting this
/// backwards swaps the funnel's notion of left and right, and the path then
/// hugs the *far* side of every corner instead of the near one — a plausible
/// looking route that is roughly twice as long as it should be.
fn area_sign(a: Vec2, b: Vec2, c: Vec2) -> i32 {
    -orient(a, b, c).sign()
}

/// Total length of a polyline path.
pub fn path_length(path: &[Vec2]) -> f64 {
    path.windows(2).map(|w| w[0].distance(w[1])).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triangulate_constrained;

    fn pts(v: &[(f64, f64)]) -> Vec<Vec2> {
        v.iter().map(|(x, y)| Vec2::new(*x, *y)).collect()
    }

    fn hall() -> NavMesh {
        let p = pts(&[(0.0, 0.0), (20.0, 0.0), (20.0, 12.0), (0.0, 12.0)]);
        let mut t = triangulate_constrained(&p, &[(0, 1), (1, 2), (2, 3), (3, 0)]).unwrap();
        t.compact();
        NavMesh::build(t)
    }

    #[test]
    fn an_empty_hall_has_one_internal_portal() {
        let m = hall();
        assert_eq!(m.portals.len(), 1, "the shared diagonal is the only portal");
        // The diagonal of a 20 x 12 rectangle.
        let expected = (20.0f64 * 20.0 + 12.0 * 12.0).sqrt();
        assert!((m.portals[0].width - expected).abs() < 1e-9);
    }

    #[test]
    fn walls_are_never_portals() {
        let m = hall();
        // Four constrained walls plus the diagonal = 5 edges; only the
        // unconstrained diagonal may be a portal.
        for p in &m.portals {
            let key = crate::edge_key(p.a, p.b);
            assert!(
                !m.tri.constraints.contains(&key),
                "portal {p:?} sits on a wall"
            );
        }
    }

    #[test]
    fn locate_finds_the_right_triangle() {
        let m = hall();
        assert!(m.locate(Vec2::new(10.0, 6.0)).is_some());
        assert!(m.locate(Vec2::new(1.0, 1.0)).is_some());
        // Outside the hall entirely.
        assert!(m.locate(Vec2::new(-5.0, 6.0)).is_none());
        assert!(m.locate(Vec2::new(25.0, 6.0)).is_none());
    }

    /// In an empty convex hall the shortest path is a straight line, and the
    /// funnel must produce exactly that — not a detour via triangle centroids.
    #[test]
    fn path_across_an_empty_hall_is_straight() {
        let m = hall();
        let from = Vec2::new(2.0, 2.0);
        let to = Vec2::new(18.0, 10.0);

        let path = m.find_path(from, to).expect("path exists");
        assert_eq!(path.first(), Some(&from));
        assert_eq!(path.last(), Some(&to));

        let straight = from.distance(to);
        assert!(
            (path_length(&path) - straight).abs() < 1e-9,
            "path length {} should equal the straight line {straight}; path = {path:?}",
            path_length(&path)
        );
    }

    #[test]
    fn path_within_a_single_triangle() {
        let m = hall();
        let from = Vec2::new(2.0, 2.0);
        let to = Vec2::new(4.0, 3.0);
        let path = m.find_path(from, to).unwrap();
        assert_eq!(path, vec![from, to]);
    }

    /// A hall with a wall spur reaching up from the south wall, leaving a gap
    /// at the top. A straight crossing is blocked; the route must go over.
    ///
    /// Note the outline traces *around* the spur as one simple ring. An earlier
    /// version of this fixture modelled the spur as a separate closed rectangle
    /// sharing its base with the south wall — which is geometrically incoherent,
    /// because that "interior" is solid wall contiguous with the outdoors and is
    /// reachable at two different nesting parities. `classify` reported it as
    /// `InconsistentNesting`, correctly.
    fn hall_with_spur() -> NavMesh {
        let p = pts(&[
            (0.0, 0.0),
            (9.0, 0.0),
            (9.0, 9.0), // up the spur's left face
            (11.0, 9.0),
            (11.0, 0.0), // down its right face
            (20.0, 0.0),
            (20.0, 12.0),
            (0.0, 12.0),
        ]);
        let edges = [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 4),
            (4, 5),
            (5, 6),
            (6, 7),
            (7, 0),
        ];
        let mut t = triangulate_constrained(&p, &edges).unwrap();
        t.compact();
        NavMesh::build(t)
    }

    /// The real test of the funnel: an obstacle forces the path to bend, and it
    /// must hug the corner rather than wander through triangle centroids.
    #[test]
    fn path_bends_around_an_obstacle() {
        let m = hall_with_spur();
        assert!(m.regions.warnings.is_empty(), "{:?}", m.regions.warnings);

        let from = Vec2::new(2.0, 4.0);
        let to = Vec2::new(18.0, 4.0);
        let path = m.find_path(from, to).expect("a route around the pillar");

        let straight = from.distance(to);
        let len = path_length(&path);

        // It must be longer than the blocked straight line...
        assert!(
            len > straight,
            "path {len} should exceed the blocked straight line {straight}"
        );
        // ...and equal to the taut route over the spur's two top corners. This
        // is the assertion that actually tests the funnel: a centroid path, or
        // one with left/right swapped, is materially longer.
        let taut = from.distance(Vec2::new(9.0, 9.0))
            + Vec2::new(9.0, 9.0).distance(Vec2::new(11.0, 9.0))
            + Vec2::new(11.0, 9.0).distance(to);
        assert!(
            (len - taut).abs() < 1e-9,
            "path {len} is not taut (expected {taut}): {path:?}"
        );

        // It must actually turn a corner.
        assert!(path.len() > 2, "expected a bend, got {path:?}");

        // No path vertex may sit inside the pillar footprint.
        for v in &path {
            let inside_pillar = v.x > 9.0 + 1e-9 && v.x < 11.0 - 1e-9 && v.y < 9.0 - 1e-9;
            assert!(!inside_pillar, "path vertex {v:?} is inside the spur");
        }
    }

    #[test]
    fn no_path_to_a_sealed_room() {
        // Two halls sharing a wall with no opening between them.
        let p = pts(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 12.0),
            (0.0, 12.0), // left room
            (12.0, 0.0),
            (22.0, 0.0),
            (22.0, 12.0),
            (12.0, 12.0), // right room
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
        let m = NavMesh::build(t);

        let left = Vec2::new(5.0, 6.0);
        let right = Vec2::new(17.0, 6.0);
        assert!(m.locate(left).is_some());
        assert!(m.locate(right).is_some());
        assert!(
            m.find_path(left, right).is_none(),
            "there is no door between these rooms"
        );
    }

    #[test]
    fn path_to_an_unreachable_point_is_none() {
        let m = hall();
        assert!(m
            .find_path(Vec2::new(2.0, 2.0), Vec2::new(100.0, 100.0))
            .is_none());
        assert!(m
            .find_path(Vec2::new(-5.0, 2.0), Vec2::new(10.0, 6.0))
            .is_none());
    }

    #[test]
    fn narrowest_portal_reports_the_tightest_gap() {
        let m = hall_with_spur();
        let narrow = m.narrowest_portal().expect("portals exist");
        assert!(narrow.width > 0.0);
        // Every portal must be at least as wide.
        for p in &m.portals {
            assert!(p.width >= narrow.width - 1e-12);
        }
    }
}
