/* eslint-disable */
/**
 * GENERATED from schema/venue.schema.json — do not edit.
 * Regenerate with: pnpm schema
 * The source of truth is engine/cf-schema (ADR 0001).
 */

/**
 * A 2D point in metres, as [x, y].
 *
 * @minItems 2
 * @maxItems 2
 */
export type Vec2 = [number, number];
export type FlowDirection = "in" | "out" | "both";
export type QueueDiscipline = "single" | "parallel" | "serpentine";
/**
 * A sampleable distribution.
 *
 * Serialised as an internally-tagged union on the `dist` field, e.g. `{"dist":"normal","mean":1.34,"sd":0.26,"min":0.6,"max":2.2}`.
 */
export type Distribution =
  | {
      dist: "constant";
      value: number;
    }
  | {
      dist: "uniform";
      max: number;
      min: number;
    }
  | {
      dist: "normal";
      max?: number | null;
      mean: number;
      min?: number | null;
      sd: number;
    }
  | {
      dist: "lognormal";
      max?: number | null;
      min?: number | null;
      muLn: number;
      sigmaLn: number;
    }
  | {
      dist: "exponential";
      lambda: number;
      max?: number | null;
    }
  | {
      dist: "categorical";
      p: {
        [k: string]: number;
      };
    }
  | {
      dist: "empirical";
      samples: number[];
    };
export type GateState = "open" | "closed" | "entryOnly" | "exitOnly";
export type ComponentType =
  | "turnstile"
  | "securityCheckpoint"
  | "registrationDesk"
  | "ticketCounter"
  | "barricade"
  | "seatingBlock"
  | "stall"
  | "sign";
export type ObstacleKind = "generic" | "pillar" | "furniture" | "equipment" | "planter" | "water";
export type OpeningKind = ("door" | "doubleDoor" | "gate" | "revolving" | "emergencyExit") | "gap";
export type Swing = "both" | "inward" | "outward";
export type WallKind = "structural" | "partition" | "barrier" | "temporary";
/**
 * NFPA 101 Table 7.3.1.2 occupancy classification.
 *
 * The occupant load factors themselves live in `cf-compliance/rules/` as data, not here — this enum only names the classification.
 */
export type ZoneKind =
  | (
      | "assemblyFixedSeating"
      | "circulation"
      | "business"
      | "mercantile"
      | "storage"
      | "backOfHouse"
      | "queue"
      | "restricted"
      | "exterior"
    )
  | "assemblyConcentrated"
  | "assemblyLessConcentrated"
  | "assemblyStandingSpace";
export type LayerKind = "normal" | "proposal";
export type LinkKind = ("stair" | "ramp" | "escalator" | "elevator") | "opening";
export type EdgeDirection = "both" | "forward" | "backward";
export type FlowConstraintKind = "oneWay" | "preferred" | "avoid";
export type CalibrationSource = "fileHeader" | "ocrDimension" | "doorWidthPrior" | "manualTwoPoint";
/**
 * Display unit only. **All stored coordinates are metres regardless.** Mixing storage units is how unit-confusion bugs get into safety calculations.
 */
export type LengthUnit = "m" | "ft";

/**
 * A complete venue.
 */
export interface VenueDoc {
  annotations?: Annotation[];
  floors: Floor[];
  georef?: Georef | null;
  id: string;
  layers?: Layer[];
  /**
   * Vertical connections between floors.
   */
  links?: Link[];
  name: string;
  provenance?: Provenance | null;
  routing?: Routing;
  scale?: ScaleCalibration | null;
  /**
   * Always `cfs.venue/1.0`. Checked on load so a future format change fails loudly rather than silently mis-parsing.
   */
  schemaVersion: string;
  units?: Units;
}
export interface Annotation {
  floor?: string | null;
  id: string;
  kind: string;
  p: Vec2;
  text?: string | null;
}
export interface Floor {
  ceilingM?: number | null;
  components?: Component[];
  elevationM?: number;
  id: string;
  name: string;
  obstacles?: Obstacle[];
  openings?: Opening[];
  walls?: Wall[];
  zones?: Zone[];
}
/**
 * A placed intelligent asset. Not a drawing — a simulation node carrying real operational metadata (throughput ceilings, service-time distributions).
 */
export interface Component {
  id: string;
  layer?: string | null;
  name?: string | null;
  params: ComponentParams;
  /**
   * Zone in which agents queue for this component.
   */
  queueArea?: string | null;
  schedule?: ScheduleEntry[];
  /**
   * Access tags this component will serve. Empty means all.
   */
  servesAccess?: string[];
  transform: Transform;
  type: ComponentType;
}
/**
 * Per-type parameters.
 *
 * Every field is optional at the schema level and validated per-type in [`crate::validate`], rather than being a tagged union. Two reasons: the editor can change a component's type without losing compatible params, and adding a param to one type does not churn the wire format for the others.
 */
export interface ComponentParams {
  /**
   * Relative pull for dwell attractors (stalls, booths).
   */
  attractorWeight?: number | null;
  cols?: number | null;
  /**
   * Fraction of unfamiliar agents that obey this sign.
   */
  complianceRate?: number | null;
  direction?: FlowDirection | null;
  /**
   * Physical footprint in floor-local metres, relative to `transform.p`.
   */
  footprint?: Vec2[] | null;
  headingDeg?: number | null;
  laneWidthM?: number | null;
  /**
   * Number of parallel lanes / stations / servers.
   */
  lanes?: number | null;
  /**
   * Hard ceiling in persons per hour. Enforced independently of the physics so a component cannot exceed its manufacturer-rated throughput.
   */
  maxThroughputPph?: number | null;
  queueDiscipline?: QueueDiscipline | null;
  radiusM?: number | null;
  rowPitchM?: number | null;
  rows?: number | null;
  seatPitchM?: number | null;
  /**
   * Probability an agent is pulled aside for secondary screening.
   */
  secondaryRate?: number | null;
  secondaryTime?: Distribution | null;
  /**
   * Per-person service time.
   */
  serviceTime?: Distribution | null;
}
/**
 * Time-varying state for an opening, component or link. Gives Core Capability 3's "time windows" — gates opening and closing during an event.
 */
export interface ScheduleEntry {
  /**
   * Simulation time in seconds at which this state takes effect.
   */
  fromS: number;
  state: GateState;
}
/**
 * Placement of a component on a floor.
 */
export interface Transform {
  /**
   * Origin in floor-local metres.
   */
  p: Vec2;
  /**
   * Rotation in degrees, counter-clockwise from +x.
   */
  rotDeg?: number;
}
export interface Obstacle {
  heightM?: number | null;
  id: string;
  kind?: ObstacleKind & string;
  layer?: string | null;
  polygon: Vec2[];
  provenance?: Provenance | null;
  traversable?: boolean;
}
/**
 * Where an element came from, and how much we trust it.
 *
 * Every element produced by import carries this. It is what lets the review UI band elements by confidence, and what lets a report state which parts of the model were machine-generated versus human-drawn.
 */
export interface Provenance {
  /**
   * 0.0–1.0. `1.0` for human-drawn or exact vector extraction.
   */
  confidence?: number;
  importJob?: string | null;
  reviewedAt?: string | null;
  reviewedBy?: string | null;
  source: string;
  sourceFile?: string | null;
}
/**
 * A door, gate or gap, stored **parametrically along its parent wall**.
 *
 * `t` is normalised arc length in `[0,1]`. Storing it this way means moving or re-snapping a wall carries its doors with it — the most common editing operation stays correct with no constraint solver.
 */
export interface Opening {
  /**
   * Multiplies the code-derived rate of passage. 1.0 unless a physical feature (revolving door, tight lobby) justifies otherwise.
   */
  capacityFactor?: number;
  id: string;
  isFireExit?: boolean;
  kind?: OpeningKind & string;
  provenance?: Provenance | null;
  schedule?: ScheduleEntry[];
  swing?: Swing & string;
  /**
   * Normalised position along the wall polyline, `0.0..=1.0`.
   */
  t: number;
  wall: string;
  widthM: number;
}
export interface Wall {
  id: string;
  kind?: WallKind & string;
  layer?: string | null;
  /**
   * Whether agents may pass through (e.g. a rope line, a low barrier).
   */
  permeable?: boolean;
  polyline: Vec2[];
  provenance?: Provenance | null;
  thicknessM?: number;
}
/**
 * A semantic area. `kind` is the hook the compliance engine reads to derive an occupant load — the author picks a meaning, the engine derives the number.
 */
export interface Zone {
  /**
   * Access tags permitted to enter. Empty means unrestricted.
   */
  access?: string[];
  attractors?: Attractor[];
  id: string;
  /**
   * A hole in the floor (atrium, stairwell shaft) — not walkable, and excluded from occupant load.
   */
  isVoid?: boolean;
  kind: ZoneKind;
  layer?: string | null;
  name?: string | null;
  olfJustification?: string | null;
  /**
   * Override the code-derived occupant load factor, in m² per person. Requires `olfJustification` — an unexplained override is exactly the kind of thing an auditor will ask about.
   */
  olfOverride?: number | null;
  polygon: Vec2[];
  provenance?: Provenance | null;
  speedMultiplier?: number;
}
/**
 * A point of interest that draws agents. Weight is relative within a floor.
 */
export interface Attractor {
  kind: string;
  point?: Vec2 | null;
  weight?: number;
}
/**
 * Optional real-world georeferencing, for venues placed on a map.
 */
export interface Georef {
  epsg: number;
  origin: Vec2;
  rotationDeg?: number;
}
export interface Layer {
  id: string;
  kind?: LayerKind & string;
  locked?: boolean;
  name: string;
  visible?: boolean;
  z?: number;
}
/**
 * A vertical connection between two floors.
 */
export interface Link {
  /**
   * Width excluding handrails — the figure egress capacity is computed from.
   */
  clearWidthM?: number | null;
  direction?: FlowDirection & string;
  /**
   * Exactly two ends, one per floor.
   */
  ends: LinkEnd[];
  /**
   * Green Guide rate of passage, persons per metre per minute. 82 on the level, 66 on stairs.
   */
  flowRatePpmm?: number | null;
  goingM?: number | null;
  id: string;
  kind: LinkKind;
  name?: string | null;
  riserM?: number | null;
  schedule?: ScheduleEntry[];
  speedMultiplierDown?: number | null;
  speedMultiplierUp?: number | null;
  steps?: number | null;
  widthM: number;
}
export interface LinkEnd {
  floor: string;
  footprint: Vec2[];
}
/**
 * The planner's explicit circulatory network, layered on top of the geometry.
 */
export interface Routing {
  edges?: RoutingEdge[];
  flowConstraints?: FlowConstraint[];
  waypoints?: Waypoint[];
}
export interface RoutingEdge {
  costMult?: number;
  direction?: EdgeDirection & string;
  from: string;
  to: string;
}
/**
 * A directional bias applied across a zone, e.g. a one-way corridor.
 */
export interface FlowConstraint {
  headingDeg?: number | null;
  kind: FlowConstraintKind;
  /**
   * Compliance fraction, 0.0–1.0. `0.85` means 15% of agents ignore the constraint — which is what actually happens with signage and stanchions.
   */
  strength?: number;
  zone: string;
}
export interface Waypoint {
  floor: string;
  id: string;
  p: Vec2;
  radiusM?: number;
}
/**
 * How the drawing's pixel/drawing units were mapped to metres during import.
 *
 * A wrong scale silently invalidates every downstream compliance number, so this records *how* it was established and how confident we are.
 */
export interface ScaleCalibration {
  calibration: CalibrationSource;
  /**
   * 0.0–1.0.
   */
  confidence?: number;
  /**
   * Set once a human has confirmed the scale. Import must not commit without it.
   */
  confirmed?: boolean;
  sourcePxPerMeter: number;
}
export interface Units {
  length?: LengthUnit & string;
}
