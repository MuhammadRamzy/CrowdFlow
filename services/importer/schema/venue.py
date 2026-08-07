# GENERATED from schema/venue.schema.json — do not edit.
# Regenerate with: make models   (from services/)
# The source of truth is engine/cf-schema (ADR 0001).

from __future__ import annotations

from typing import Annotated, Literal

from pydantic import BaseModel, Field, RootModel


class Distribution1(BaseModel):
    """
    A fixed value. Useful for pinning a parameter during sensitivity analysis.
    """

    dist: Literal["constant"]
    value: float


class Distribution2(BaseModel):
    """
    A sampleable distribution.

    Serialised as an internally-tagged union on the `dist` field, e.g. `{"dist":"normal","mean":1.34,"sd":0.26,"min":0.6,"max":2.2}`.
    """

    dist: Literal["uniform"]
    max: float
    min: float


class Distribution3(BaseModel):
    """
    A sampleable distribution.

    Serialised as an internally-tagged union on the `dist` field, e.g. `{"dist":"normal","mean":1.34,"sd":0.26,"min":0.6,"max":2.2}`.
    """

    dist: Literal["normal"]
    max: float | None = None
    mean: float
    min: float | None = None
    sd: float


class Distribution4(BaseModel):
    """
    Parameterised by the mean and sd **of the underlying normal**, not of the resulting variate. Named `muLn`/`sigmaLn` rather than `mu`/`sigma` so the distinction is impossible to miss at a call site — getting this wrong is the single most common modelling error with lognormals.

    Note: `rename_all` on the enum renames *variants*, not their fields, so multi-word fields need their own attribute.
    """

    dist: Literal["lognormal"]
    max: float | None = None
    min: float | None = None
    muLn: float
    sigmaLn: float


class Distribution5(BaseModel):
    """
    A sampleable distribution.

    Serialised as an internally-tagged union on the `dist` field, e.g. `{"dist":"normal","mean":1.34,"sd":0.26,"min":0.6,"max":2.2}`.
    """

    dist: Literal["exponential"]
    lambda_: Annotated[float, Field(alias="lambda")]
    max: float | None = None


class Distribution6(BaseModel):
    """
    Discrete outcomes with weights. Weights need not sum to 1; they are normalised. Keys are strings so that group sizes (`"1"`, `"2"`, …) and named categories share one representation.
    """

    dist: Literal["categorical"]
    p: dict[str, float]


class Distribution7(BaseModel):
    """
    Empirical distribution over observed samples, sampled by linear interpolation of the order statistics.
    """

    dist: Literal["empirical"]
    samples: list[float]


class Provenance(BaseModel):
    """
    Where an element came from, and how much we trust it.

    Every element produced by import carries this. It is what lets the review UI band elements by confidence, and what lets a report state which parts of the model were machine-generated versus human-drawn.
    """

    confidence: float | None = 1.0
    """
    0.0–1.0. `1.0` for human-drawn or exact vector extraction.
    """
    importJob: str | None = None
    reviewedAt: str | None = None
    reviewedBy: str | None = None
    source: str
    sourceFile: str | None = None


class RoutingEdge(BaseModel):
    costMult: float | None = 1.0
    direction: Literal["both", "forward", "backward"] | None = "both"
    from_: Annotated[str, Field(alias="from")]
    to: str


class ScaleCalibration(BaseModel):
    """
    How the drawing's pixel/drawing units were mapped to metres during import.

    A wrong scale silently invalidates every downstream compliance number, so this records *how* it was established and how confident we are.
    """

    calibration: (
        Literal["fileHeader"]
        | Literal["ocrDimension"]
        | Literal["doorWidthPrior"]
        | Literal["manualTwoPoint"]
    )
    confidence: float | None = 0.0
    """
    0.0–1.0.
    """
    confirmed: bool | None = False
    """
    Set once a human has confirmed the scale. Import must not commit without it.
    """
    sourcePxPerMeter: float


class ScheduleEntry(BaseModel):
    """
    Time-varying state for an opening, component or link. Gives Core Capability 3's "time windows" — gates opening and closing during an event.
    """

    fromS: float
    """
    Simulation time in seconds at which this state takes effect.
    """
    state: Literal["open", "closed", "entryOnly", "exitOnly"]


class Units(BaseModel):
    length: Literal["m", "ft"] | None = "m"
    """
    Display unit only. **All stored coordinates are metres regardless.** Mixing storage units is how unit-confusion bugs get into safety calculations.
    """


class Vec2(RootModel[list[float]]):
    """
    A 2D point in metres, as [x, y].
    """

    root: Annotated[list[float], Field(max_length=2, min_length=2)]
    """
    A 2D point in metres, as [x, y].
    """


class Waypoint(BaseModel):
    floor: str
    id: str
    p: Annotated[list[float], Field(max_length=2, min_length=2)]
    """
    A 2D point in metres, as [x, y].
    """
    radiusM: float | None = 1.5


class Annotation(BaseModel):
    floor: str | None = None
    id: str
    kind: str
    p: Annotated[list[float], Field(max_length=2, min_length=2)]
    """
    A 2D point in metres, as [x, y].
    """
    text: str | None = None


class Attractor(BaseModel):
    """
    A point of interest that draws agents. Weight is relative within a floor.
    """

    kind: str
    point: Vec2 | None = None
    weight: float | None = 1.0


class ComponentParams(BaseModel):
    """
    Per-type parameters.

    Every field is optional at the schema level and validated per-type in [`crate::validate`], rather than being a tagged union. Two reasons: the editor can change a component's type without losing compatible params, and adding a param to one type does not churn the wire format for the others.
    """

    attractorWeight: float | None = None
    """
    Relative pull for dwell attractors (stalls, booths).
    """
    cols: Annotated[int | None, Field(ge=0)] = None
    complianceRate: float | None = None
    """
    Fraction of unfamiliar agents that obey this sign.
    """
    direction: Literal["in", "out", "both"] | None = None
    footprint: list[Vec2] | None = None
    """
    Physical footprint in floor-local metres, relative to `transform.p`.
    """
    headingDeg: float | None = None
    laneWidthM: float | None = None
    lanes: Annotated[int | None, Field(ge=0)] = None
    """
    Number of parallel lanes / stations / servers.
    """
    maxThroughputPph: float | None = None
    """
    Hard ceiling in persons per hour. Enforced independently of the physics so a component cannot exceed its manufacturer-rated throughput.
    """
    queueDiscipline: (
        Literal["single"] | Literal["parallel"] | Literal["serpentine"] | None
    ) = None
    radiusM: float | None = None
    rowPitchM: float | None = None
    rows: Annotated[int | None, Field(ge=0)] = None
    seatPitchM: float | None = None
    secondaryRate: float | None = None
    """
    Probability an agent is pulled aside for secondary screening.
    """
    secondaryTime: (
        Distribution1
        | Distribution2
        | Distribution3
        | Distribution4
        | Distribution5
        | Distribution6
        | Distribution7
        | None
    ) = None
    serviceTime: (
        Distribution1
        | Distribution2
        | Distribution3
        | Distribution4
        | Distribution5
        | Distribution6
        | Distribution7
        | None
    ) = None
    """
    Per-person service time.
    """


class FlowConstraint(BaseModel):
    """
    A directional bias applied across a zone, e.g. a one-way corridor.
    """

    headingDeg: float | None = None
    kind: Literal["oneWay", "preferred", "avoid"]
    strength: float | None = 0.85
    """
    Compliance fraction, 0.0–1.0. `0.85` means 15% of agents ignore the constraint — which is what actually happens with signage and stanchions.
    """
    zone: str


class Georef(BaseModel):
    """
    Optional real-world georeferencing, for venues placed on a map.
    """

    epsg: Annotated[int, Field(ge=0)]
    origin: Annotated[list[float], Field(max_length=2, min_length=2)]
    """
    A 2D point in metres, as [x, y].
    """
    rotationDeg: float | None = 0.0


class Layer(BaseModel):
    id: str
    kind: Literal["normal"] | Literal["proposal"] | None = "normal"
    locked: bool | None = False
    name: str
    visible: bool | None = True
    z: int | None = 0


class LinkEnd(BaseModel):
    floor: str
    footprint: list[Vec2]


class Obstacle(BaseModel):
    heightM: float | None = None
    id: str
    kind: (
        Literal["generic", "pillar", "furniture", "equipment", "planter", "water"]
        | None
    ) = "generic"
    layer: str | None = None
    polygon: list[Vec2]
    provenance: Provenance | None = None
    traversable: bool | None = False


class Opening(BaseModel):
    """
    A door, gate or gap, stored **parametrically along its parent wall**.

    `t` is normalised arc length in `[0,1]`. Storing it this way means moving or re-snapping a wall carries its doors with it — the most common editing operation stays correct with no constraint solver.
    """

    capacityFactor: float | None = 1.0
    """
    Multiplies the code-derived rate of passage. 1.0 unless a physical feature (revolving door, tight lobby) justifies otherwise.
    """
    id: str
    isFireExit: bool | None = False
    kind: (
        Literal["door", "doubleDoor", "gate", "revolving", "emergencyExit"]
        | Literal["gap"]
        | None
    ) = "door"
    provenance: Provenance | None = None
    schedule: list[ScheduleEntry] | None = None
    swing: Literal["both", "inward", "outward"] | None = "both"
    t: float
    """
    Normalised position along the wall polyline, `0.0..=1.0`.
    """
    wall: str
    widthM: float


class Routing(BaseModel):
    """
    The planner's explicit circulatory network, layered on top of the geometry.
    """

    edges: list[RoutingEdge] | None = None
    flowConstraints: list[FlowConstraint] | None = None
    waypoints: list[Waypoint] | None = None


class Transform(BaseModel):
    """
    Placement of a component on a floor.
    """

    p: Annotated[list[float], Field(max_length=2, min_length=2)]
    """
    Origin in floor-local metres.
    """
    rotDeg: float | None = 0.0
    """
    Rotation in degrees, counter-clockwise from +x.
    """


class Wall(BaseModel):
    id: str
    kind: Literal["structural", "partition", "barrier", "temporary"] | None = (
        "structural"
    )
    layer: str | None = None
    permeable: bool | None = False
    """
    Whether agents may pass through (e.g. a rope line, a low barrier).
    """
    polyline: list[Vec2]
    provenance: Provenance | None = None
    thicknessM: float | None = 0.2


class Zone(BaseModel):
    """
    A semantic area. `kind` is the hook the compliance engine reads to derive an occupant load — the author picks a meaning, the engine derives the number.
    """

    access: list[str] | None = None
    """
    Access tags permitted to enter. Empty means unrestricted.
    """
    attractors: list[Attractor] | None = None
    id: str
    isVoid: bool | None = False
    """
    A hole in the floor (atrium, stairwell shaft) — not walkable, and excluded from occupant load.
    """
    kind: (
        Literal[
            "assemblyFixedSeating",
            "circulation",
            "business",
            "mercantile",
            "storage",
            "backOfHouse",
            "queue",
            "restricted",
            "exterior",
        ]
        | Literal["assemblyConcentrated"]
        | Literal["assemblyLessConcentrated"]
        | Literal["assemblyStandingSpace"]
    )
    """
    NFPA 101 Table 7.3.1.2 occupancy classification.

    The occupant load factors themselves live in `cf-compliance/rules/` as data, not here — this enum only names the classification.
    """
    layer: str | None = None
    name: str | None = None
    olfJustification: str | None = None
    olfOverride: float | None = None
    """
    Override the code-derived occupant load factor, in m² per person. Requires `olfJustification` — an unexplained override is exactly the kind of thing an auditor will ask about.
    """
    polygon: list[Vec2]
    provenance: Provenance | None = None
    speedMultiplier: float | None = 1.0


class Component(BaseModel):
    """
    A placed intelligent asset. Not a drawing — a simulation node carrying real operational metadata (throughput ceilings, service-time distributions).
    """

    id: str
    layer: str | None = None
    name: str | None = None
    params: ComponentParams
    queueArea: str | None = None
    """
    Zone in which agents queue for this component.
    """
    schedule: list[ScheduleEntry] | None = None
    servesAccess: list[str] | None = None
    """
    Access tags this component will serve. Empty means all.
    """
    transform: Transform
    type: Literal[
        "turnstile",
        "securityCheckpoint",
        "registrationDesk",
        "ticketCounter",
        "barricade",
        "seatingBlock",
        "stall",
        "sign",
    ]


class Floor(BaseModel):
    ceilingM: float | None = None
    components: list[Component] | None = None
    elevationM: float | None = 0.0
    id: str
    name: str
    obstacles: list[Obstacle] | None = None
    openings: list[Opening] | None = None
    walls: list[Wall] | None = None
    zones: list[Zone] | None = None


class Link(BaseModel):
    """
    A vertical connection between two floors.
    """

    clearWidthM: float | None = None
    """
    Width excluding handrails — the figure egress capacity is computed from.
    """
    direction: Literal["in", "out", "both"] | None = "both"
    ends: list[LinkEnd]
    """
    Exactly two ends, one per floor.
    """
    flowRatePpmm: float | None = None
    """
    Green Guide rate of passage, persons per metre per minute. 82 on the level, 66 on stairs.
    """
    goingM: float | None = None
    id: str
    kind: Literal["stair", "ramp", "escalator", "elevator"] | Literal["opening"]
    name: str | None = None
    riserM: float | None = None
    schedule: list[ScheduleEntry] | None = None
    speedMultiplierDown: float | None = None
    speedMultiplierUp: float | None = None
    steps: Annotated[int | None, Field(ge=0)] = None
    widthM: float


class VenueDoc(BaseModel):
    """
    A complete venue.
    """

    annotations: list[Annotation] | None = None
    floors: list[Floor]
    georef: Georef | None = None
    id: str
    layers: list[Layer] | None = None
    links: list[Link] | None = None
    """
    Vertical connections between floors.
    """
    name: str
    provenance: Provenance | None = None
    routing: Annotated[Routing | None, Field(validate_default=True)] = {}
    scale: ScaleCalibration | None = None
    schemaVersion: str
    """
    Always `cfs.venue/1.0`. Checked on load so a future format change fails loudly rather than silently mis-parsing.
    """
    units: Annotated[Units | None, Field(validate_default=True)] = {"length": "m"}
