//! Diagnostics the compiler emits for the editor's validation panel.
//!
//! This is the feedback channel that makes the compile step feel like a feature
//! rather than a wall (`docs/03-track-a-venue-designer.md` §A2). Every warning
//! names the element it concerns so the canvas can pan to it.

use cf_schema::ids::{FloorId, LinkId, OpeningId, WallId, ZoneId};

#[derive(Clone, Debug, PartialEq)]
pub enum CompileWarning {
    /// A wall has fewer than two distinct points after deduplication.
    DegenerateWall { wall: WallId, floor: FloorId },

    /// An opening references a wall that does not exist on its floor.
    OpeningOrphaned { opening: OpeningId, wall: WallId },

    /// An opening is wider than the wall it sits in, so its span was clamped.
    OpeningOverflowsWall {
        opening: OpeningId,
        width_m: f64,
        wall_length_m: f64,
    },

    /// An opening's clear width is below the egress minimum.
    OpeningTooNarrow {
        opening: OpeningId,
        width_m: f64,
        minimum_m: f64,
    },

    /// Two openings on the same wall overlap; their spans were merged.
    OpeningsOverlap {
        a: OpeningId,
        b: OpeningId,
        wall: WallId,
    },

    /// The floor produced no walkable area. Almost always an unclosed outline.
    NoWalkableArea { floor: FloorId },

    /// Constraint rings are not closed, so inside/outside is ambiguous here.
    UnclosedOutline { floor: FloorId, detail: String },

    /// Part of the mesh cannot be reached from the rest of it.
    DisconnectedRegion { floor: FloorId, triangles: usize },

    /// A zone's centroid does not land on walkable floor.
    ZoneNotOnFloor { zone: ZoneId, floor: FloorId },

    /// A zone asks for a walking-speed change that no triangle picked up.
    ///
    /// The multiplier is applied to triangles whose centroid the zone contains.
    /// A zone smaller than the local triangles contains none, so the stair or
    /// ramp it describes would be walked at full speed — silently, which is
    /// worse than not supporting it at all.
    ZoneSpeedNotApplied { zone: ZoneId, floor: FloorId },

    /// A vertical link could not be resolved to a route between two floors.
    ///
    /// The staircase is in the drawing and not in the model. That is worth
    /// saying loudly: an egress analysis that quietly loses a route reports a
    /// building it is not describing.
    LinkNotUsable { link: LinkId, detail: String },

    /// The floor has no opening marked as a fire exit.
    NoFireExit { floor: FloorId },

    /// Triangulation failed outright; the floor has no mesh.
    TriangulationFailed { floor: FloorId, detail: String },

    /// A floor had no wall geometry at all.
    EmptyFloor { floor: FloorId },
}

impl CompileWarning {
    /// Does this stop the floor being simulated?
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            CompileWarning::TriangulationFailed { .. }
                | CompileWarning::NoWalkableArea { .. }
                | CompileWarning::EmptyFloor { .. }
        )
    }
}

impl std::fmt::Display for CompileWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use CompileWarning::*;
        match self {
            DegenerateWall { wall, floor } => {
                write!(f, "wall '{wall}' on floor '{floor}' has no length")
            }
            OpeningOrphaned { opening, wall } => {
                write!(f, "opening '{opening}' references unknown wall '{wall}'")
            }
            OpeningOverflowsWall {
                opening,
                width_m,
                wall_length_m,
            } => write!(
                f,
                "opening '{opening}' is {width_m:.2} m wide but its wall is only \
                 {wall_length_m:.2} m long; the span was clamped"
            ),
            OpeningTooNarrow {
                opening,
                width_m,
                minimum_m,
            } => write!(
                f,
                "opening '{opening}' has {width_m:.2} m clear width, below the \
                 {minimum_m:.2} m egress minimum"
            ),
            OpeningsOverlap { a, b, wall } => write!(
                f,
                "openings '{a}' and '{b}' overlap on wall '{wall}'; their spans were merged"
            ),
            NoWalkableArea { floor } => write!(
                f,
                "floor '{floor}' has no walkable area; its outline is probably not closed"
            ),
            UnclosedOutline { floor, detail } => {
                write!(f, "floor '{floor}' has an unclosed wall run: {detail}")
            }
            DisconnectedRegion { floor, triangles } => write!(
                f,
                "floor '{floor}' has {triangles} triangle(s) unreachable from the rest"
            ),
            LinkNotUsable { link, detail } => write!(
                f,
                "link '{link}' cannot be used as a route between floors: {detail}"
            ),
            ZoneSpeedNotApplied { zone, floor } => write!(
                f,
                "zone '{zone}' on floor '{floor}' sets a walking-speed multiplier \
                 but is too small for the mesh here, so no triangle takes it and \
                 the speed change will not happen"
            ),
            ZoneNotOnFloor { zone, floor } => write!(
                f,
                "zone '{zone}' on floor '{floor}' does not sit on walkable floor"
            ),
            NoFireExit { floor } => {
                write!(f, "floor '{floor}' has no opening marked as a fire exit")
            }
            TriangulationFailed { floor, detail } => {
                write!(f, "floor '{floor}' could not be triangulated: {detail}")
            }
            EmptyFloor { floor } => write!(f, "floor '{floor}' has no wall geometry"),
        }
    }
}
