//! The Venue Document — authored, editable, human-facing geometry and semantics.
//!
//! This is the source of truth for a venue. It is *not* what the simulation
//! consumes: `cf-compile` turns it into a `NavGraph` (see
//! docs/01-architecture.md §1). Keeping the two separate is the load-bearing
//! decision of the whole architecture — the editor never has to understand
//! triangulation, and the engine never has to understand undo stacks.

use crate::dist::Distribution;
use crate::geom::{Polygon, Polyline, Transform, Vec2};
use crate::ids::*;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const VENUE_SCHEMA_VERSION: &str = "cfs.venue/1.0";

/// A complete venue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VenueDoc {
    /// Always `cfs.venue/1.0`. Checked on load so a future format change fails
    /// loudly rather than silently mis-parsing.
    pub schema_version: String,
    pub id: VenueId,
    pub name: String,

    #[serde(default)]
    pub units: Units,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub georef: Option<Georef>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<ScaleCalibration>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<Layer>,

    pub floors: Vec<Floor>,

    /// Vertical connections between floors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,

    #[serde(default)]
    pub routing: Routing,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

impl VenueDoc {
    /// A minimal valid document: one empty floor, no geometry.
    pub fn empty(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            schema_version: VENUE_SCHEMA_VERSION.to_string(),
            id: VenueId::new(id),
            name: name.into(),
            units: Units::default(),
            georef: None,
            scale: None,
            layers: vec![Layer::default_structure()],
            floors: vec![Floor::empty("f0", "Ground", 0.0)],
            links: Vec::new(),
            routing: Routing::default(),
            annotations: Vec::new(),
            provenance: None,
        }
    }

    pub fn floor(&self, id: &FloorId) -> Option<&Floor> {
        self.floors.iter().find(|f| &f.id == id)
    }

    /// Total gross floor area in m², summed over every zone on every floor.
    pub fn total_zone_area(&self) -> f64 {
        self.floors
            .iter()
            .flat_map(|f| f.zones.iter())
            .filter(|z| !z.is_void)
            .map(|z| z.polygon.area())
            .sum()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Units {
    #[serde(default = "default_length_unit")]
    pub length: LengthUnit,
}

impl Default for Units {
    fn default() -> Self {
        Self {
            length: LengthUnit::Meter,
        }
    }
}

fn default_length_unit() -> LengthUnit {
    LengthUnit::Meter
}

/// Display unit only. **All stored coordinates are metres regardless.** Mixing
/// storage units is how unit-confusion bugs get into safety calculations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum LengthUnit {
    #[serde(rename = "m")]
    Meter,
    #[serde(rename = "ft")]
    Foot,
}

/// Optional real-world georeferencing, for venues placed on a map.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Georef {
    pub epsg: u32,
    pub origin: Vec2,
    #[serde(default)]
    pub rotation_deg: f64,
}

/// How the drawing's pixel/drawing units were mapped to metres during import.
///
/// A wrong scale silently invalidates every downstream compliance number, so
/// this records *how* it was established and how confident we are.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScaleCalibration {
    pub source_px_per_meter: f64,
    pub calibration: CalibrationSource,
    /// 0.0–1.0.
    #[serde(default)]
    pub confidence: f64,
    /// Set once a human has confirmed the scale. Import must not commit without it.
    #[serde(default)]
    pub confirmed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CalibrationSource {
    /// DXF `$INSUNITS` header or equivalent explicit declaration.
    FileHeader,
    /// OCR'd dimension string cross-checked against the geometry it annotates.
    OcrDimension,
    /// Inferred from the modal detected door width (~0.9 m).
    DoorWidthPrior,
    /// User clicked two points and typed the real distance.
    ManualTwoPoint,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub z: i32,
    #[serde(default)]
    pub kind: LayerKind,
}

impl Layer {
    pub fn default_structure() -> Self {
        Self {
            id: LayerId::new("lay_struct"),
            name: "Structure".into(),
            visible: true,
            locked: false,
            z: 0,
            kind: LayerKind::Normal,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum LayerKind {
    #[default]
    Normal,
    /// Un-committed AI or import output awaiting human review. Nothing on a
    /// proposal layer is ever simulated or reported until accepted.
    Proposal,
}

fn yes() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Floors
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Floor {
    pub id: FloorId,
    pub name: String,
    #[serde(default)]
    pub elevation_m: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ceiling_m: Option<f64>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub walls: Vec<Wall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub openings: Vec<Opening>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<Zone>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub obstacles: Vec<Obstacle>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<Component>,
}

impl Floor {
    pub fn empty(id: &str, name: &str, elevation_m: f64) -> Self {
        Self {
            id: FloorId::new(id),
            name: name.to_owned(),
            elevation_m,
            ceiling_m: None,
            walls: Vec::new(),
            openings: Vec::new(),
            zones: Vec::new(),
            obstacles: Vec::new(),
            components: Vec::new(),
        }
    }

    pub fn wall(&self, id: &WallId) -> Option<&Wall> {
        self.walls.iter().find(|w| &w.id == id)
    }

    pub fn zone(&self, id: &ZoneId) -> Option<&Zone> {
        self.zones.iter().find(|z| &z.id == id)
    }

    /// Resolve an opening's parametric position to world coordinates.
    pub fn opening_position(&self, op: &Opening) -> Option<Vec2> {
        self.wall(&op.wall)?.polyline.point_at(op.t)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Wall {
    pub id: WallId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<LayerId>,
    pub polyline: Polyline,
    #[serde(default = "default_wall_thickness")]
    pub thickness_m: f64,
    #[serde(default)]
    pub kind: WallKind,
    /// Whether agents may pass through (e.g. a rope line, a low barrier).
    #[serde(default)]
    pub permeable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

fn default_wall_thickness() -> f64 {
    0.20
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum WallKind {
    #[default]
    Structural,
    Partition,
    Barrier,
    Temporary,
}

/// A door, gate or gap, stored **parametrically along its parent wall**.
///
/// `t` is normalised arc length in `[0,1]`. Storing it this way means moving or
/// re-snapping a wall carries its doors with it — the most common editing
/// operation stays correct with no constraint solver.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Opening {
    pub id: OpeningId,
    pub wall: WallId,
    /// Normalised position along the wall polyline, `0.0..=1.0`.
    pub t: f64,
    pub width_m: f64,
    #[serde(default)]
    pub kind: OpeningKind,
    #[serde(default)]
    pub swing: Swing,
    #[serde(default)]
    pub is_fire_exit: bool,
    /// Multiplies the code-derived rate of passage. 1.0 unless a physical
    /// feature (revolving door, tight lobby) justifies otherwise.
    #[serde(default = "one")]
    pub capacity_factor: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedule: Vec<ScheduleEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

fn one() -> f64 {
    1.0
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum OpeningKind {
    #[default]
    Door,
    DoubleDoor,
    /// A plain gap in a wall — no leaf, no frame.
    Gap,
    Gate,
    Revolving,
    EmergencyExit,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Swing {
    #[default]
    Both,
    Inward,
    Outward,
}

/// Time-varying state for an opening, component or link. Gives Core Capability
/// 3's "time windows" — gates opening and closing during an event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleEntry {
    /// Simulation time in seconds at which this state takes effect.
    pub from_s: f64,
    pub state: GateState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum GateState {
    #[default]
    Open,
    Closed,
    EntryOnly,
    ExitOnly,
}

/// A semantic area. `kind` is the hook the compliance engine reads to derive an
/// occupant load — the author picks a meaning, the engine derives the number.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Zone {
    pub id: ZoneId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<LayerId>,
    pub polygon: Polygon,
    pub kind: ZoneKind,
    /// Override the code-derived occupant load factor, in m² per person.
    /// Requires `olfJustification` — an unexplained override is exactly the kind
    /// of thing an auditor will ask about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub olf_override: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub olf_justification: Option<String>,
    /// Access tags permitted to enter. Empty means unrestricted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub access: Vec<String>,
    #[serde(default = "one")]
    pub speed_multiplier: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attractors: Vec<Attractor>,
    /// A hole in the floor (atrium, stairwell shaft) — not walkable, and
    /// excluded from occupant load.
    #[serde(default)]
    pub is_void: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

/// NFPA 101 Table 7.3.1.2 occupancy classification.
///
/// The occupant load factors themselves live in `cf-compliance/rules/` as data,
/// not here — this enum only names the classification.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ZoneKind {
    /// Standing, dance floor, no fixed seating. OLF 0.65 m²/person.
    AssemblyConcentrated,
    /// Dining, exhibition. OLF 1.4 m²/person.
    AssemblyLessConcentrated,
    /// Standing space. OLF 0.46 m²/person.
    AssemblyStandingSpace,
    AssemblyFixedSeating,
    Circulation,
    Business,
    Mercantile,
    Storage,
    BackOfHouse,
    Queue,
    Restricted,
    Exterior,
}

/// A point of interest that draws agents. Weight is relative within a floor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Attractor {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<Vec2>,
    #[serde(default = "one")]
    pub weight: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Obstacle {
    pub id: ObstacleId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<LayerId>,
    pub polygon: Polygon,
    #[serde(default)]
    pub kind: ObstacleKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height_m: Option<f64>,
    #[serde(default)]
    pub traversable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ObstacleKind {
    #[default]
    Generic,
    Pillar,
    Furniture,
    Equipment,
    Planter,
    Water,
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// A placed intelligent asset. Not a drawing — a simulation node carrying real
/// operational metadata (throughput ceilings, service-time distributions).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    pub id: ComponentId,
    #[serde(rename = "type")]
    pub component_type: ComponentType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<LayerId>,
    pub transform: Transform,
    pub params: ComponentParams,
    /// Zone in which agents queue for this component.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_area: Option<ZoneId>,
    /// Access tags this component will serve. Empty means all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub serves_access: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedule: Vec<ScheduleEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ComponentType {
    Turnstile,
    SecurityCheckpoint,
    RegistrationDesk,
    TicketCounter,
    Barricade,
    SeatingBlock,
    Stall,
    Sign,
}

/// Per-type parameters.
///
/// Every field is optional at the schema level and validated per-type in
/// [`crate::validate`], rather than being a tagged union. Two reasons: the
/// editor can change a component's type without losing compatible params, and
/// adding a param to one type does not churn the wire format for the others.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ComponentParams {
    /// Number of parallel lanes / stations / servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lanes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane_width_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<FlowDirection>,
    /// Per-person service time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_time: Option<Distribution>,
    /// Hard ceiling in persons per hour. Enforced independently of the physics
    /// so a component cannot exceed its manufacturer-rated throughput.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_throughput_pph: Option<f64>,
    /// Probability an agent is pulled aside for secondary screening.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_time: Option<Distribution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_discipline: Option<QueueDiscipline>,
    /// Physical footprint in floor-local metres, relative to `transform.p`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<Polygon>,
    // Seating blocks
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cols: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seat_pitch_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_pitch_m: Option<f64>,
    // Signs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_deg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_m: Option<f64>,
    /// Fraction of unfamiliar agents that obey this sign.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compliance_rate: Option<f64>,
    /// Relative pull for dwell attractors (stalls, booths).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attractor_weight: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum FlowDirection {
    #[serde(rename = "in")]
    In,
    #[serde(rename = "out")]
    Out,
    #[default]
    Both,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum QueueDiscipline {
    /// One line per server.
    #[default]
    Parallel,
    /// One line feeding all servers.
    Serpentine,
    Single,
}

// ---------------------------------------------------------------------------
// Vertical links
// ---------------------------------------------------------------------------

/// A vertical connection between two floors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Link {
    pub id: LinkId,
    pub kind: LinkKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Exactly two ends, one per floor.
    pub ends: Vec<LinkEnd>,
    pub width_m: f64,
    /// Width excluding handrails — the figure egress capacity is computed from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clear_width_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub riser_m: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub going_m: Option<f64>,
    #[serde(default)]
    pub direction: FlowDirection,
    /// Green Guide rate of passage, persons per metre per minute.
    /// 82 on the level, 66 on stairs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_rate_ppmm: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_multiplier_up: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_multiplier_down: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedule: Vec<ScheduleEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LinkEnd {
    pub floor: FloorId,
    pub footprint: Polygon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum LinkKind {
    Stair,
    Ramp,
    Escalator,
    Elevator,
    /// A void with no physical connection — used to model an atrium edge.
    Opening,
}

// ---------------------------------------------------------------------------
// Routing
// ---------------------------------------------------------------------------

/// The planner's explicit circulatory network, layered on top of the geometry.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Routing {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waypoints: Vec<Waypoint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<RoutingEdge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flow_constraints: Vec<FlowConstraint>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Waypoint {
    pub id: WaypointId,
    pub floor: FloorId,
    pub p: Vec2,
    #[serde(default = "default_waypoint_radius")]
    pub radius_m: f64,
}

fn default_waypoint_radius() -> f64 {
    1.5
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoutingEdge {
    pub from: WaypointId,
    pub to: WaypointId,
    #[serde(default)]
    pub direction: EdgeDirection,
    #[serde(default = "one")]
    pub cost_mult: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum EdgeDirection {
    #[default]
    Both,
    Forward,
    Backward,
}

/// A directional bias applied across a zone, e.g. a one-way corridor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowConstraint {
    pub zone: ZoneId,
    pub kind: FlowConstraintKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading_deg: Option<f64>,
    /// Compliance fraction, 0.0–1.0. `0.85` means 15% of agents ignore the
    /// constraint — which is what actually happens with signage and stanchions.
    #[serde(default = "default_strength")]
    pub strength: f64,
}

fn default_strength() -> f64 {
    0.85
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum FlowConstraintKind {
    OneWay,
    Preferred,
    Avoid,
}

// ---------------------------------------------------------------------------
// Annotations and provenance
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    pub id: AnnotationId,
    pub kind: String,
    pub p: Vec2,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<FloorId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// Where an element came from, and how much we trust it.
///
/// Every element produced by import carries this. It is what lets the review UI
/// band elements by confidence, and what lets a report state which parts of the
/// model were machine-generated versus human-drawn.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import_job: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    /// 0.0–1.0. `1.0` for human-drawn or exact vector extraction.
    #[serde(default = "one")]
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<String>,
}
