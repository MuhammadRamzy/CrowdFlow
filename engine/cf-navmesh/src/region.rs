//! Deciding which triangles are floor and which are solid.
//!
//! A constrained triangulation tiles the whole convex hull. That includes the
//! space outside the building and the inside of every pillar — geometry that
//! exists in the mesh but that nobody can stand on. Region classification
//! separates the two.
//!
//! # The rule
//!
//! Start outside and count constraint crossings. Every closed ring of
//! constrained edges — the floor outline, a pillar outline — flips you between
//! solid and open:
//!
//! ```text
//!   depth 0   outside the building        solid
//!   depth 1   inside the outline          WALKABLE
//!   depth 2   inside a pillar             solid
//!   depth 3   a courtyard within a pillar WALKABLE
//! ```
//!
//! So **odd depth is walkable**. A min-depth flood from the convex hull —
//! incrementing across constrained edges, preserving across ordinary ones —
//! assigns this in one pass.
//!
//! The flood relaxes to the *minimum* crossing count rather than taking
//! whichever path arrives first. That matters when a wall run has a gap: two
//! routes then reach the same triangle with different parity, and a plain
//! breadth-first fill would silently pick one. Taking the minimum makes the
//! answer well defined, and a separate consistency pass reports the
//! disagreement rather than hiding it.
//!
//! # What this assumes
//!
//! Constraint rings must be closed. A wall run with a gap in it lets the fill
//! leak through, and the interior is classified solid — which is exactly the
//! symptom of an unclosed room and precisely what the import pipeline's
//! topology repair exists to prevent. [`classify`] reports leaks it can detect
//! so `cf-compile` can surface them as `CompileWarning`s rather than silently
//! producing a venue with no floor.

use crate::triangulation::{TriIdx, Triangulation, NO_NEIGHBOUR};

/// Per-triangle classification for a mesh.
#[derive(Clone, Debug, Default)]
pub struct Regions {
    /// Nesting depth, indexed by triangle. `u32::MAX` for triangles the fill
    /// never reached (only possible in a disconnected mesh).
    pub depth: Vec<u32>,
    /// `true` where an agent can stand — odd nesting depth.
    pub walkable: Vec<bool>,
    /// Diagnostics for `cf-compile` to surface in the editor.
    pub warnings: Vec<RegionWarning>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegionWarning {
    /// No triangle touched the convex hull, so there was nothing to seed from.
    NoBoundary,
    /// A triangle was unreachable from the hull. The mesh is disconnected.
    Unreachable { triangle: TriIdx },
    /// Nothing was classified walkable. Almost always an unclosed outline
    /// letting the exterior fill leak inward.
    NoWalkableArea,
    /// Two adjacent triangles disagree about nesting depth, meaning "inside"
    /// is not well defined here. The constraint rings are not closed.
    InconsistentNesting { a: TriIdx, b: TriIdx },
}

impl std::fmt::Display for RegionWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegionWarning::NoBoundary => write!(f, "mesh has no boundary edge to seed from"),
            RegionWarning::Unreachable { triangle } => {
                write!(
                    f,
                    "triangle {triangle} is unreachable from the mesh boundary"
                )
            }
            RegionWarning::NoWalkableArea => write!(
                f,
                "no walkable area found; the floor outline is probably not closed"
            ),
            RegionWarning::InconsistentNesting { a, b } => write!(
                f,
                "triangles {a} and {b} disagree about inside/outside; a wall run is not closed"
            ),
        }
    }
}

impl Regions {
    pub fn is_walkable(&self, t: TriIdx) -> bool {
        self.walkable.get(t).copied().unwrap_or(false)
    }

    pub fn walkable_count(&self) -> usize {
        self.walkable.iter().filter(|w| **w).count()
    }
}

/// Classify every triangle as floor or solid.
///
/// Call on a compacted triangulation with adjacency built.
///
/// ```
/// use cf_navmesh::{triangulate_constrained, region};
/// use cf_geom::Vec2;
///
/// // A 20 x 12 hall.
/// let pts = vec![
///     Vec2::new(0.0, 0.0),
///     Vec2::new(20.0, 0.0),
///     Vec2::new(20.0, 12.0),
///     Vec2::new(0.0, 12.0),
/// ];
/// let mut t = triangulate_constrained(&pts, &[(0, 1), (1, 2), (2, 3), (3, 0)]).unwrap();
/// t.compact();
///
/// let r = region::classify(&t);
/// assert_eq!(r.walkable_count(), 2);
/// assert!((region::walkable_area(&t, &r) - 240.0).abs() < 1e-9);
/// ```
pub fn classify(t: &Triangulation) -> Regions {
    let n = t.triangles.len();
    let mut depth = vec![u32::MAX; n];
    let mut warnings = Vec::new();
    let mut queue = std::collections::BinaryHeap::new();

    // Seed from the convex hull. The true exterior is the unbounded region
    // *outside* the hull, at depth 0 — it has no triangles of its own.
    //
    // So a hull triangle's depth depends on whether its hull edge is a wall:
    // reaching it across a constrained hull edge means one crossing has already
    // happened (depth 1, inside), across an open one means none (depth 0).
    //
    // This matters because the common case — a rectangular hall whose outline
    // *is* its convex hull — has no exterior triangles at all. Seeding every
    // hull triangle at 0 would classify the entire venue as solid.
    for (idx, tri) in t.live() {
        let mut seed: Option<u32> = None;
        for i in 0..3 {
            if tri.n[i] != NO_NEIGHBOUR {
                continue;
            }
            let d = if tri.constrained[i] { 1 } else { 0 };
            seed = Some(seed.map_or(d, |s: u32| s.min(d)));
        }
        if let Some(d) = seed {
            depth[idx] = d;
            queue.push(std::cmp::Reverse((d, idx)));
        }
    }

    if queue.is_empty() {
        warnings.push(RegionWarning::NoBoundary);
    }

    // Relax to the *minimum* crossing count rather than taking whichever path
    // arrives first. With a leak, two routes to the same triangle disagree
    // about parity, and plain BFS would silently pick one. Processing in
    // increasing depth gives a well-defined answer, and the inconsistency
    // check below then reports the disagreement instead of hiding it.
    while let Some(std::cmp::Reverse((d, cur))) = queue.pop() {
        if d > depth[cur] {
            continue; // stale entry
        }
        for i in 0..3 {
            let nb = t.triangles[cur].n[i];
            if nb == NO_NEIGHBOUR || t.triangles[nb].deleted {
                continue;
            }
            // Crossing a wall changes which side of it you are on.
            let next = if t.triangles[cur].constrained[i] {
                d + 1
            } else {
                d
            };
            if next < depth[nb] {
                depth[nb] = next;
                queue.push(std::cmp::Reverse((next, nb)));
            }
        }
    }

    for (idx, _) in t.live() {
        if depth[idx] == u32::MAX {
            warnings.push(RegionWarning::Unreachable { triangle: idx });
        }
    }

    // Consistency: across an open edge the nesting depth must be unchanged;
    // across a wall it must differ by exactly one. Any violation means the
    // constraint rings are not closed, so "inside" is not well defined.
    // This is the precise signature of an unrepaired import.
    let mut reported = std::collections::HashSet::new();
    for (idx, tri) in t.live() {
        if depth[idx] == u32::MAX {
            continue;
        }
        for i in 0..3 {
            let nb = tri.n[i];
            if nb == NO_NEIGHBOUR || t.triangles[nb].deleted || depth[nb] == u32::MAX {
                continue;
            }
            let expected_gap = u32::from(tri.constrained[i]);
            let actual_gap = depth[idx].abs_diff(depth[nb]);
            if actual_gap != expected_gap {
                let key = if idx < nb { (idx, nb) } else { (nb, idx) };
                if reported.insert(key) {
                    warnings.push(RegionWarning::InconsistentNesting { a: key.0, b: key.1 });
                }
            }
        }
    }

    let walkable: Vec<bool> = (0..n)
        .map(|i| !t.triangles[i].deleted && depth[i] != u32::MAX && depth[i] % 2 == 1)
        .collect();

    if !walkable.iter().any(|w| *w) && t.live_count() > 0 {
        warnings.push(RegionWarning::NoWalkableArea);
    }

    Regions {
        depth,
        walkable,
        warnings,
    }
}

/// Total walkable floor area in m².
///
/// This is the figure NFPA occupant load is computed from, so it must exclude
/// obstacle interiors — which is the whole point of classification.
pub fn walkable_area(t: &Triangulation, r: &Regions) -> f64 {
    t.live()
        .filter(|(idx, _)| r.is_walkable(*idx))
        .map(|(_, tri)| {
            let [a, b, c] = tri.v;
            cf_geom::predicates::signed_area(t.points[a], t.points[b], t.points[c])
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triangulate_constrained;
    use cf_geom::Vec2;

    fn pts(v: &[(f64, f64)]) -> Vec<Vec2> {
        v.iter().map(|(x, y)| Vec2::new(*x, *y)).collect()
    }

    #[test]
    fn a_closed_hall_is_entirely_walkable() {
        let p = pts(&[(0.0, 0.0), (20.0, 0.0), (20.0, 12.0), (0.0, 12.0)]);
        let mut t = triangulate_constrained(&p, &[(0, 1), (1, 2), (2, 3), (3, 0)]).unwrap();
        t.compact();

        let r = classify(&t);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert_eq!(r.walkable_count(), 2);
        assert!((walkable_area(&t, &r) - 240.0).abs() < 1e-9);
    }

    /// The case classification exists for: a pillar's interior is inside the
    /// building outline but is not floor.
    #[test]
    fn a_pillar_interior_is_not_walkable() {
        let p = pts(&[
            (0.0, 0.0),
            (20.0, 0.0),
            (20.0, 12.0),
            (0.0, 12.0),
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

        let r = classify(&t);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);

        // Hall minus the 2 x 2 pillar.
        let area = walkable_area(&t, &r);
        assert!(
            (area - (240.0 - 4.0)).abs() < 1e-9,
            "walkable area {area} should be 236"
        );

        // The whole mesh still tiles the rectangle; only the classification
        // differs. That distinction is the point.
        assert!((t.total_area() - 240.0).abs() < 1e-9);

        // Depth 2 exists — the pillar interior.
        assert!(
            r.depth.contains(&2),
            "expected triangles nested inside the pillar"
        );
    }

    /// A courtyard inside a solid block: depth 3, walkable again.
    #[test]
    fn nesting_alternates_correctly() {
        let p = pts(&[
            // Outer building
            (0.0, 0.0),
            (30.0, 0.0),
            (30.0, 30.0),
            (0.0, 30.0),
            // Solid block inside it
            (8.0, 8.0),
            (22.0, 8.0),
            (22.0, 22.0),
            (8.0, 22.0),
            // Courtyard inside the block
            (12.0, 12.0),
            (18.0, 12.0),
            (18.0, 18.0),
            (12.0, 18.0),
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
            (8, 9),
            (9, 10),
            (10, 11),
            (11, 8),
        ];
        let mut t = triangulate_constrained(&p, &edges).unwrap();
        t.compact();

        let r = classify(&t);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);

        // Walkable = ring between outer and block (900 - 196), plus the
        // courtyard (36).
        let expected = (900.0 - 196.0) + 36.0;
        let area = walkable_area(&t, &r);
        assert!(
            (area - expected).abs() < 1e-9,
            "walkable area {area} should be {expected}"
        );
        assert!(r.depth.contains(&3), "courtyard depth missing");
    }

    /// An unclosed outline lets the exterior leak in. The result is no walkable
    /// area, and the warning says so — this is what an unrepaired import looks
    /// like, and it must be reported rather than shipped as a venue with no floor.
    #[test]
    fn an_unclosed_outline_is_reported() {
        let p = pts(&[(0.0, 0.0), (20.0, 0.0), (20.0, 12.0), (0.0, 12.0)]);
        // Three walls out of four — a gap along the top.
        let mut t = triangulate_constrained(&p, &[(0, 1), (1, 2), (3, 0)]).unwrap();
        t.compact();

        let r = classify(&t);
        assert!(
            r.warnings.contains(&RegionWarning::NoWalkableArea),
            "expected a leak warning, got {:?}",
            r.warnings
        );
        assert_eq!(r.walkable_count(), 0);
    }

    #[test]
    fn unconstrained_mesh_has_no_floor() {
        // No constraints at all: everything is depth 0, so nothing is walkable.
        let p = pts(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
        let mut t = triangulate_constrained(&p, &[]).unwrap();
        t.compact();

        let r = classify(&t);
        assert_eq!(r.walkable_count(), 0);
        assert!(r.warnings.contains(&RegionWarning::NoWalkableArea));
    }

    /// The M1 fixture geometry: a hall whose south wall is split by two door
    /// openings. The doorways are gaps, so the outline is *not* closed — which
    /// is correct, and the interior stays connected to the exterior through
    /// them. This documents the behaviour cf-compile must account for: door
    /// gaps need sealing with a virtual edge before classification.
    #[test]
    fn door_gaps_leak_until_sealed() {
        // South wall split around two 1.8 m doorways.
        let p = pts(&[
            (0.0, 0.0),   // 0
            (4.1, 0.0),   // 1  door 1 start
            (5.9, 0.0),   // 2  door 1 end
            (14.1, 0.0),  // 3  door 2 start
            (15.9, 0.0),  // 4  door 2 end
            (20.0, 0.0),  // 5
            (20.0, 12.0), // 6
            (0.0, 12.0),  // 7
        ]);
        let walls = [
            (0, 1),
            (2, 3),
            (4, 5), // south wall, doorways omitted
            (5, 6),
            (6, 7),
            (7, 0),
        ];

        let mut t = triangulate_constrained(&p, &walls).unwrap();
        t.compact();
        let leaky = classify(&t);
        assert_eq!(
            leaky.walkable_count(),
            0,
            "open doorways should let the exterior fill leak in"
        );

        // Sealing the doorways with virtual edges closes the outline.
        let sealed_walls = [
            (0, 1),
            (1, 2), // door 1 sealed
            (2, 3),
            (3, 4), // door 2 sealed
            (4, 5),
            (5, 6),
            (6, 7),
            (7, 0),
        ];
        let mut t2 = triangulate_constrained(&p, &sealed_walls).unwrap();
        t2.compact();
        let sealed = classify(&t2);

        assert!(sealed.warnings.is_empty(), "{:?}", sealed.warnings);
        assert!(
            (walkable_area(&t2, &sealed) - 240.0).abs() < 1e-9,
            "sealed hall should be fully walkable, got {}",
            walkable_area(&t2, &sealed)
        );
    }
}
