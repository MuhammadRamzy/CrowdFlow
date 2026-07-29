//! # cf-navmesh
//!
//! Triangulation and navigation-mesh construction for CrowdFlow Studio.
//!
//! The walkable region of a floor — its boundary minus obstacles, with openings
//! left as gaps — is triangulated into a mesh that agents path across. See
//! `docs/04-track-b-simulation-engine.md` §B1 for why a triangulation rather
//! than a grid: venues have long diagonal and curved walls (stadium bowls,
//! concourses), which a grid either aliases or needs a punishing cell size to
//! represent. A CDT gives exact boundaries with far fewer cells, and the funnel
//! algorithm then yields geometrically optimal paths.
//!
//! ## Status
//!
//! - [`triangulation`] — unconstrained Delaunay (Bowyer–Watson). **Done.**
//! - [`constraint`] — constraint edge insertion. **Done.**
//! - [`region`] — walkable vs solid classification. **Done.**
//! - [`navmesh`] — portals, A* corridor + funnel pathfinding. **Done.**
//! - Mesh refinement and flow fields — next.
//!
//! Constraint insertion is deliberately a separate step built on a verified
//! unconstrained triangulation. Writing both at once makes failures impossible
//! to attribute: a bad mesh could be the cavity logic or the constraint walk,
//! and the symptoms look identical.

pub mod constraint;
pub mod navmesh;
pub mod region;
pub mod triangulation;

pub use constraint::{insert_constraint, triangulate_constrained, CdtError, ConstraintError};
pub use navmesh::{path_length, NavMesh, Portal};
pub use region::{classify, walkable_area, RegionWarning, Regions};
pub use triangulation::{
    edge_key, triangulate, TriIdx, Triangle, Triangulation, TriangulationError, VertIdx,
    NO_NEIGHBOUR,
};
