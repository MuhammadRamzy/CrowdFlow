//! # cf-compile
//!
//! Turns an authored [`VenueDoc`] into a simulation-ready [`NavGraph`].
//!
//! This is the boundary between Track A and Track B (`docs/01-architecture.md`
//! §1). The editor never has to understand triangulation; the engine never has
//! to understand undo stacks. Everything crosses here.
//!
//! ## The pipeline, per floor
//!
//! 1. Cut each wall centreline around its openings ([`walls::split_wall`]).
//! 2. Accumulate points with deduplication ([`points::PointSet`]) — the
//!    triangulator rejects coincident points rather than dropping them.
//! 3. Constrain the solid wall runs and every obstacle ring.
//! 4. **Seal each doorway with a temporary constraint**, then triangulate.
//! 5. Classify walkable regions.
//! 6. **Remove the doorway seals** and rebuild adjacency, so the doorways
//!    become ordinary mesh edges again.
//! 7. Record each doorway as a [`DoorNode`] against the interior triangle
//!    behind it.
//!
//! ## Why step 4 exists
//!
//! A door is a *gap* in a wall run. Without sealing, the floor outline is not a
//! closed ring, region classification's fill leaks straight in from outside,
//! and the entire venue is classified as solid — zero walkable area. Sealing,
//! classifying, then unsealing gets both: a correctly enclosed interior *and*
//! doorways agents can actually reach.
//!
//! ## Walls are centrelines, not solids
//!
//! Wall `thicknessM` does not inflate the mesh. Agents carry a radius and the
//! Social Force Model applies wall repulsion, so clearance is handled by the
//! physics rather than by eroding the navigable region. This keeps the mesh far
//! simpler — inflating every wall would require unioning overlapping rings at
//! every corner. `cf_geom::offset_polyline_to_ring` exists for when thickness
//! needs to become real geometry (rendering, area take-offs).

pub mod points;
pub mod walls;
pub mod warning;

use cf_geom::Vec2;
use cf_navmesh::{region, triangulate_constrained, NavMesh, TriIdx, VertIdx};
use cf_schema::ids::{FloorId, OpeningId};
use cf_schema::venue::Floor;
use cf_schema::VenueDoc;

pub use points::PointSet;
pub use walls::{split_wall, DoorGap, WallSplit};
pub use warning::CompileWarning;

/// Bumped whenever a change alters compiled output. Recorded in every run so a
/// report can state exactly what produced it.
pub const COMPILER_VERSION: &str = "cf-compile/0.1.0";

/// NFPA 101 requires 32 in (0.813 m) clear egress width; we flag below 0.85 m
/// to leave margin for door hardware.
pub const MIN_EGRESS_WIDTH_M: f64 = 0.85;

/// A doorway in the compiled mesh.
#[derive(Clone, Debug)]
pub struct DoorNode {
    pub opening: OpeningId,
    pub a: Vec2,
    pub b: Vec2,
    pub width_m: f64,
    pub is_fire_exit: bool,
    /// The walkable triangle immediately inside this doorway. `None` means the
    /// doorway does not border walkable floor — usually a door in an interior
    /// wall of a region that is itself solid.
    pub inside: Option<TriIdx>,
}

impl DoorNode {
    pub fn midpoint(&self) -> Vec2 {
        self.a.lerp(self.b, 0.5)
    }
}

/// One compiled floor.
#[derive(Clone, Debug)]
pub struct FloorMesh {
    pub floor: FloorId,
    pub mesh: NavMesh,
    pub doors: Vec<DoorNode>,
}

impl FloorMesh {
    /// Total walkable floor area in m².
    pub fn walkable_area(&self) -> f64 {
        region::walkable_area(&self.mesh.tri, &self.mesh.regions)
    }

    pub fn fire_exits(&self) -> impl Iterator<Item = &DoorNode> {
        self.doors.iter().filter(|d| d.is_fire_exit)
    }
}

/// The compiled artifact the simulation consumes.
#[derive(Clone, Debug)]
pub struct NavGraph {
    pub compiler_version: &'static str,
    pub floors: Vec<FloorMesh>,
    pub warnings: Vec<CompileWarning>,
}

impl NavGraph {
    pub fn floor(&self, id: &FloorId) -> Option<&FloorMesh> {
        self.floors.iter().find(|f| &f.floor == id)
    }

    /// Warnings that prevent simulation.
    pub fn fatal_warnings(&self) -> impl Iterator<Item = &CompileWarning> {
        self.warnings.iter().filter(|w| w.is_fatal())
    }

    pub fn is_simulable(&self) -> bool {
        !self.floors.is_empty() && self.fatal_warnings().count() == 0
    }

    pub fn total_walkable_area(&self) -> f64 {
        self.floors.iter().map(|f| f.walkable_area()).sum()
    }
}

/// Compile a venue.
///
/// Always returns a graph. Floors that could not be built are absent from
/// `floors` and explained in `warnings` — a partial result plus diagnostics is
/// more useful to the editor than an error that loses everything.
pub fn compile(doc: &VenueDoc) -> NavGraph {
    let mut warnings = Vec::new();
    let mut floors = Vec::new();

    for floor in &doc.floors {
        if let Some(fm) = compile_floor(floor, &mut warnings) {
            floors.push(fm);
        }
    }

    NavGraph {
        compiler_version: COMPILER_VERSION,
        floors,
        warnings,
    }
}

/// Compile a single floor. Pushes diagnostics onto `warnings`.
pub fn compile_floor(floor: &Floor, warnings: &mut Vec<CompileWarning>) -> Option<FloorMesh> {
    if floor.walls.is_empty() {
        warnings.push(CompileWarning::EmptyFloor {
            floor: floor.id.clone(),
        });
        return None;
    }

    let mut pts = PointSet::new();
    let mut wall_edges: Vec<(VertIdx, VertIdx)> = Vec::new();
    let mut seal_edges: Vec<(VertIdx, VertIdx)> = Vec::new();
    let mut doors: Vec<DoorNode> = Vec::new();

    // --- walls, cut around their openings -------------------------------
    for wall in &floor.walls {
        let openings: Vec<&_> = floor
            .openings
            .iter()
            .filter(|o| o.wall == wall.id)
            .collect();

        let split = split_wall(wall, &openings);

        if split.solid.is_empty() && split.gaps.is_empty() {
            warnings.push(CompileWarning::DegenerateWall {
                wall: wall.id.clone(),
                floor: floor.id.clone(),
            });
            continue;
        }

        for id in &split.clamped {
            let op = openings.iter().find(|o| &o.id == id);
            warnings.push(CompileWarning::OpeningOverflowsWall {
                opening: id.clone(),
                width_m: op.map(|o| o.width_m).unwrap_or(0.0),
                wall_length_m: wall.polyline.length(),
            });
        }
        for (a, b) in &split.overlaps {
            warnings.push(CompileWarning::OpeningsOverlap {
                a: a.clone(),
                b: b.clone(),
                wall: wall.id.clone(),
            });
        }

        for run in &split.solid {
            let idx: Vec<VertIdx> = run.iter().map(|p| pts.insert(*p)).collect();
            for w in idx.windows(2) {
                if w[0] != w[1] {
                    wall_edges.push((w[0], w[1]));
                }
            }
        }

        for gap in &split.gaps {
            let ia = pts.insert(gap.a);
            let ib = pts.insert(gap.b);
            if ia == ib {
                continue;
            }
            seal_edges.push((ia, ib));

            let op = openings.iter().find(|o| o.id == gap.opening);
            if gap.width_m < MIN_EGRESS_WIDTH_M {
                warnings.push(CompileWarning::OpeningTooNarrow {
                    opening: gap.opening.clone(),
                    width_m: gap.width_m,
                    minimum_m: MIN_EGRESS_WIDTH_M,
                });
            }
            doors.push(DoorNode {
                opening: gap.opening.clone(),
                a: gap.a,
                b: gap.b,
                width_m: gap.width_m,
                is_fire_exit: op.map(|o| o.is_fire_exit).unwrap_or(false),
                inside: None,
            });
        }
    }

    // Openings whose wall does not exist.
    for op in &floor.openings {
        if !floor.walls.iter().any(|w| w.id == op.wall) {
            warnings.push(CompileWarning::OpeningOrphaned {
                opening: op.id.clone(),
                wall: op.wall.clone(),
            });
        }
    }

    // --- obstacles ------------------------------------------------------
    for obs in &floor.obstacles {
        if obs.traversable || obs.polygon.len() < 3 {
            continue;
        }
        let idx: Vec<VertIdx> = obs
            .polygon
            .points()
            .iter()
            .map(|p| pts.insert(*p))
            .collect();
        for i in 0..idx.len() {
            let (a, b) = (idx[i], idx[(i + 1) % idx.len()]);
            if a != b {
                wall_edges.push((a, b));
            }
        }
    }

    if pts.len() < 3 {
        warnings.push(CompileWarning::EmptyFloor {
            floor: floor.id.clone(),
        });
        return None;
    }

    // --- triangulate, with doorways sealed ------------------------------
    let mut all_edges = wall_edges.clone();
    all_edges.extend_from_slice(&seal_edges);

    let mut tri = match triangulate_constrained(pts.points(), &all_edges) {
        Ok(t) => t,
        Err(e) => {
            warnings.push(CompileWarning::TriangulationFailed {
                floor: floor.id.clone(),
                detail: e.to_string(),
            });
            return None;
        }
    };
    tri.compact();

    // --- classify while the outline is still closed ---------------------
    let regions = region::classify(&tri);
    for w in &regions.warnings {
        match w {
            region::RegionWarning::NoWalkableArea => {
                warnings.push(CompileWarning::NoWalkableArea {
                    floor: floor.id.clone(),
                })
            }
            region::RegionWarning::InconsistentNesting { .. } => {
                warnings.push(CompileWarning::UnclosedOutline {
                    floor: floor.id.clone(),
                    detail: w.to_string(),
                })
            }
            region::RegionWarning::Unreachable { .. } => {}
            region::RegionWarning::NoBoundary => {}
        }
    }
    let unreachable = regions
        .warnings
        .iter()
        .filter(|w| matches!(w, region::RegionWarning::Unreachable { .. }))
        .count();
    if unreachable > 0 {
        warnings.push(CompileWarning::DisconnectedRegion {
            floor: floor.id.clone(),
            triangles: unreachable,
        });
    }

    // --- unseal the doorways --------------------------------------------
    // Classification is done, so the doors can become ordinary mesh edges and
    // let agents through.
    for (a, b) in &seal_edges {
        tri.constraints.remove(&cf_navmesh::edge_key(*a, *b));
    }
    tri.rebuild_adjacency();

    let mesh = NavMesh::with_regions(tri, regions);

    // --- bind each doorway to the floor behind it ------------------------
    for door in &mut doors {
        door.inside = find_inside_triangle(&mesh, door.a, door.b);
    }

    if !doors.is_empty() && !doors.iter().any(|d| d.is_fire_exit) {
        warnings.push(CompileWarning::NoFireExit {
            floor: floor.id.clone(),
        });
    }

    // --- zones must sit on floor -----------------------------------------
    for zone in &floor.zones {
        if zone.is_void {
            continue;
        }
        let Some(c) = zone.polygon.centroid() else {
            continue;
        };
        if mesh.locate(c).is_none() {
            warnings.push(CompileWarning::ZoneNotOnFloor {
                zone: zone.id.clone(),
                floor: floor.id.clone(),
            });
        }
    }

    Some(FloorMesh {
        floor: floor.id.clone(),
        mesh,
        doors,
    })
}

/// The walkable triangle holding the edge `a—b`.
fn find_inside_triangle(mesh: &NavMesh, a: Vec2, b: Vec2) -> Option<TriIdx> {
    mesh.tri.live().find_map(|(idx, t)| {
        if !mesh.regions.is_walkable(idx) {
            return None;
        }
        let has_edge = (0..3).any(|i| {
            let (x, y) = t.edge(i);
            let (px, py) = (mesh.tri.points[x], mesh.tri.points[y]);
            (px == a && py == b) || (px == b && py == a)
        });
        has_edge.then_some(idx)
    })
}
