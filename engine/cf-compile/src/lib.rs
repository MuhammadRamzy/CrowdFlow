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
use cf_schema::ids::{FloorId, LinkId, OpeningId};
use cf_schema::venue::{Floor, LinkKind};
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
    /// Stairs, ramps and lifts, resolved to a landing point on each floor.
    pub links: Vec<LinkNode>,
    pub warnings: Vec<CompileWarning>,
}

/// A vertical connection, compiled to something the simulation can walk to.
///
/// The authored `Link` carries a *footprint polygon* per end, which is the
/// right thing to draw and the wrong thing to route to. This resolves each
/// footprint to a single landing point on walkable floor, which is what a
/// waypoint needs to be.
#[derive(Clone, Debug)]
pub struct LinkNode {
    pub id: LinkId,
    pub kind: LinkKind,
    /// Index into `NavGraph::floors`, with the landing point on that floor.
    pub ends: [LinkLanding; 2],
    /// Clear width, metres — the figure egress capacity comes from, falling
    /// back to the nominal width when no clear width is given.
    pub clear_width_m: f64,
    /// Walking-speed multiplier going up, and going down.
    ///
    /// Defaulted from the Green Guide's stair figures when the document does
    /// not say: 66 persons/m/min on stairs against 82 on the level is a ratio
    /// of about 0.8, and stairs are slower down than up only marginally.
    pub speed_up: f64,
    pub speed_down: f64,
}

/// One end of a link: which floor, and where on it people arrive.
#[derive(Clone, Copy, Debug)]
pub struct LinkLanding {
    pub floor: usize,
    pub point: Vec2,
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

    let links = compile_links(doc, &floors, &mut warnings);

    NavGraph {
        compiler_version: COMPILER_VERSION,
        floors,
        links,
        warnings,
    }
}

/// Resolve authored links to landing points the simulation can route to.
///
/// A link whose end lands off walkable floor, or names a floor that did not
/// compile, is dropped with a warning rather than silently omitted. A staircase
/// that exists in the drawing and not in the model is exactly the kind of gap
/// that makes an egress analysis wrong in the optimistic direction — the
/// building has a route the report does not know about, or worse, the report
/// assumes one that is not modelled.
fn compile_links(
    doc: &VenueDoc,
    floors: &[FloorMesh],
    warnings: &mut Vec<CompileWarning>,
) -> Vec<LinkNode> {
    let mut out = Vec::new();

    for link in &doc.links {
        // A void is a hole in the floor, not a way between floors.
        if link.kind == LinkKind::Opening {
            continue;
        }
        if link.ends.len() != 2 {
            warnings.push(CompileWarning::LinkNotUsable {
                link: link.id.clone(),
                detail: format!("{} end(s); a link needs exactly two", link.ends.len()),
            });
            continue;
        }

        let mut landings = Vec::with_capacity(2);
        for end in &link.ends {
            let Some(fi) = floors.iter().position(|f| f.floor == end.floor) else {
                warnings.push(CompileWarning::LinkNotUsable {
                    link: link.id.clone(),
                    detail: format!("floor '{}' has no compiled mesh", end.floor),
                });
                break;
            };
            let mesh = &floors[fi].mesh;
            let Some(p) = end
                .footprint
                .centroid()
                .filter(|c| mesh.locate(*c).is_some())
                .or_else(|| walkable_point_in_polygon(mesh, &end.footprint))
            else {
                warnings.push(CompileWarning::LinkNotUsable {
                    link: link.id.clone(),
                    detail: format!("its end on floor '{}' is not on walkable floor", end.floor),
                });
                break;
            };
            landings.push(LinkLanding {
                floor: fi,
                point: p,
            });
        }

        if landings.len() != 2 {
            continue;
        }
        // Green Guide: 66 persons/m/min on stairs against 82 on the level.
        const STAIR_RATIO: f64 = 66.0 / 82.0;
        out.push(LinkNode {
            id: link.id.clone(),
            kind: link.kind,
            ends: [landings[0], landings[1]],
            clear_width_m: link.clear_width_m.unwrap_or(link.width_m),
            speed_up: link.speed_multiplier_up.unwrap_or(STAIR_RATIO),
            speed_down: link.speed_multiplier_down.unwrap_or(STAIR_RATIO),
        });
    }

    out
}

/// The centroid of the walkable triangle nearest the middle of `poly`.
///
/// A footprint centroid can fall in a hole — a stairwell drawn around its own
/// void is the ordinary case, not a pathological one — so this falls back to
/// somewhere a person could actually stand.
fn walkable_point_in_polygon(mesh: &NavMesh, poly: &cf_schema::Polygon) -> Option<Vec2> {
    (0..mesh.centroids.len())
        .filter(|i| mesh.regions.is_walkable(*i))
        .map(|i| mesh.centroids[i])
        .find(|c| cf_geom::polygon_ops::contains_point(poly, *c))
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

    // --- refine the mesh where a zone changes walking speed ---------------
    //
    // A zone stamps its multiplier onto triangles whose centroid it contains,
    // so a zone smaller than the local triangles takes none of them. A 20 x 12
    // hall triangulates to a handful of triangles; a 4 m stair band across it
    // contains no centroid at all, and the stair silently does not happen.
    //
    // Inserting the zone's own corners as vertices refines the triangulation
    // there, so the boundary has something to align to. They go in as **points
    // only, never as constrained edges** — a constraint is a wall in this
    // codebase, and turning a zone outline into one would seal a stair off from
    // the room it is in.
    //
    // Only zones that actually change speed. Refining around every zone would
    // grow the mesh for zones whose boundary the simulation does not care
    // about, and mesh size is the budget that decides how many agents fit.
    for zone in &floor.zones {
        if zone.is_void || (zone.speed_multiplier - 1.0).abs() < 1e-9 {
            continue;
        }
        for v in zone.polygon.points() {
            pts.insert(*v);
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

    let mut mesh = NavMesh::with_regions(tri, regions);

    // --- bind each doorway to the floor behind it ------------------------
    for door in &mut doors {
        door.inside = find_inside_triangle(&mesh, door.a, door.b);
    }

    if !doors.is_empty() && !doors.iter().any(|d| d.is_fire_exit) {
        warnings.push(CompileWarning::NoFireExit {
            floor: floor.id.clone(),
        });
    }

    // --- zones must sit on floor, and may slow the people crossing them ---
    //
    // A zone carrying a `speed_multiplier` other than 1 stamps it onto every
    // triangle whose centroid it contains. That is what turns a stair, a ramp
    // or a deliberately slow queueing area from a field the document stores
    // into something the simulation acts on.
    //
    // By centroid, so a triangle belongs wholly to one zone or to none. A
    // triangle straddling a zone edge is a meshing artefact — the zone boundary
    // ought to have been a constraint — and splitting its multiplier would give
    // an agent a speed no zone actually specifies.
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

        if (zone.speed_multiplier - 1.0).abs() < 1e-9 {
            continue;
        }
        let m = zone.speed_multiplier as f32;
        let inside: Vec<usize> = (0..mesh.centroids.len())
            .filter(|i| mesh.regions.is_walkable(*i))
            .filter(|i| cf_geom::polygon_ops::contains_point(&zone.polygon, mesh.centroids[*i]))
            .collect();
        if inside.is_empty() {
            warnings.push(CompileWarning::ZoneSpeedNotApplied {
                zone: zone.id.clone(),
                floor: floor.id.clone(),
            });
        }
        for i in inside {
            mesh.set_triangle_speed(i, m);
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
